use axum::{
    extract::State,
    http::{
        StatusCode,
        header::{AUTHORIZATION, COOKIE, SET_COOKIE},
    },
    middleware::Next,
    response::{IntoResponse, Json},
};
use log::error;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tauri::AppHandle;
use tokio::sync::{RwLock, broadcast};

/// Live browser sessions, each stamped with the moment it was issued so it can
/// be expired server-side. A cookie `Max-Age` is only a hint to the browser —
/// a captured token stays valid forever unless the server also forgets it.
pub type SessionStore = Arc<RwLock<HashMap<String, Instant>>>;

/// Must match the cookie `Max-Age` below.
pub const SESSION_TTL: Duration = Duration::from_secs(86_400);

/// Upper bound on concurrently valid sessions, so a caller with valid Basic
/// credentials cannot grow the map without limit.
const MAX_SESSIONS: usize = 1_024;

/// Shared state for web server handlers
#[derive(Clone)]
pub struct WebServerState {
    pub app_handle: AppHandle,
    pub event_tx: Arc<broadcast::Sender<TauriEvent>>,
    pub auth_credentials: Option<(String, String)>,
    pub sessions: SessionStore,
    /// Set when the server is serving TLS, so the session cookie can be marked
    /// `Secure`. Marking it `Secure` on a plain-HTTP deployment would stop the
    /// browser from ever sending it back.
    pub secure_cookies: bool,
}

/// Event message for SSE
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TauriEvent {
    pub event: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn error(message: String) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(message),
        }
    }
}

/// Custom error type for API handlers
#[derive(Debug)]
pub enum AppError {
    BadRequest(anyhow::Error),
    InternalServerError(anyhow::Error),
    NotFound(String),
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(err: E) -> Self {
        Self::InternalServerError(err.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            AppError::BadRequest(e) => (StatusCode::BAD_REQUEST, e.to_string()),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            AppError::InternalServerError(e) => {
                error!("API Error: {e:#}");
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            }
        };
        (status, Json(ApiResponse::<()>::error(message))).into_response()
    }
}

pub async fn create_session_handler(State(state): State<WebServerState>) -> impl IntoResponse {
    let token = uuid::Uuid::new_v4().simple().to_string();
    let now = Instant::now();
    {
        let mut sessions = state.sessions.write().await;
        sessions.retain(|_, issued| now.duration_since(*issued) < SESSION_TTL);
        if sessions.len() >= MAX_SESSIONS {
            // Drop the oldest so a burst of logins cannot grow the map forever.
            if let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, issued)| **issued)
                .map(|(token, _)| token.clone())
            {
                sessions.remove(&oldest);
            }
        }
        sessions.insert(token.clone(), now);
    }

    let secure = if state.secure_cookies { " Secure;" } else { "" };
    let ttl = SESSION_TTL.as_secs();
    let cookie =
        format!("session={token}; HttpOnly;{secure} SameSite=Strict; Path=/; Max-Age={ttl}");

    ([(SET_COOKIE, cookie)], Json(ApiResponse::<()>::success(())))
}

/// Deletes the session cookie, effectively logging the user out.
pub async fn delete_session_handler(
    State(state): State<WebServerState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if let Some(token) = extract_session_cookie(&headers) {
        state.sessions.write().await.remove(token);
    }

    let secure = if state.secure_cookies { " Secure;" } else { "" };
    let expire = format!("session=; HttpOnly;{secure} SameSite=Strict; Path=/; Max-Age=0");
    ([(SET_COOKIE, expire)], Json(ApiResponse::<()>::success(())))
}

pub async fn auth_middleware(
    State(state): State<WebServerState>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Result<axum::response::Response, StatusCode> {
    let Some((_, expected_creds)) = &state.auth_credentials else {
        return Ok(next.run(request).await);
    };

    if let Some(auth_header) = request.headers().get(AUTHORIZATION)
        && let Ok(auth_str) = auth_header.to_str()
        && let Some(creds) = auth_str.strip_prefix("Basic ")
        && constant_time_eq(creds.as_bytes(), expected_creds.as_bytes())
    {
        return Ok(next.run(request).await);
    }

    if let Some(token) = extract_session_cookie(request.headers()) {
        let fresh = state
            .sessions
            .read()
            .await
            .get(token)
            .is_some_and(|issued| issued.elapsed() < SESSION_TTL);
        if fresh {
            return Ok(next.run(request).await);
        }
        // Expired (or unknown): drop it so the map does not keep dead tokens.
        state.sessions.write().await.remove(token);
    }

    Ok(axum::http::Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("WWW-Authenticate", "Basic realm=\"RClone Manager\"")
        .body(axum::body::Body::from("Unauthorized"))
        .unwrap())
}

/// Compare two byte strings without leaking where they first differ. The Basic
/// credential check is the only gate on the whole `/api` surface and is
/// unauthenticated-reachable, so it must not short-circuit on the first mismatch.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn extract_session_cookie(headers: &axum::http::HeaderMap) -> Option<&str> {
    let cookie_str = headers.get(COOKIE)?.to_str().ok()?;
    cookie_str
        .split(';')
        .find_map(|part| part.trim().strip_prefix("session="))
}
