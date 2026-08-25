//! Connectivity checks for Preferences and the title-bar banner.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkCheck {
    pub url: String,
    pub ok: bool,
    pub detail: String,
}

pub fn check_links(urls: &[String], timeout_secs: u64) -> Vec<LinkCheck> {
    urls.iter()
        .map(|url| check_link(url, timeout_secs))
        .collect()
}

pub fn check_link(url: &str, timeout_secs: u64) -> LinkCheck {
    if url.trim().is_empty() {
        return LinkCheck {
            url: url.to_string(),
            ok: false,
            detail: "empty URL".into(),
        };
    }
    let timeout = Duration::from_secs(timeout_secs.max(1));
    match ureq::get(url).timeout(timeout).call() {
        Ok(resp) => LinkCheck {
            url: url.to_string(),
            ok: resp.status() < 500,
            detail: format!("HTTP {}", resp.status()),
        },
        Err(ureq::Error::Status(code, _)) => LinkCheck {
            url: url.to_string(),
            ok: code < 500,
            detail: format!("HTTP {code}"),
        },
        Err(e) => LinkCheck {
            url: url.to_string(),
            ok: false,
            detail: e.to_string(),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionStatus {
    #[default]
    Online,
    Offline,
    Checking,
}

pub fn summarize(results: &[LinkCheck]) -> String {
    let ok = results.iter().filter(|r| r.ok).count();
    format!("{ok}/{} reachable", results.len())
}

pub fn status_from_results(results: &[LinkCheck]) -> ConnectionStatus {
    if results.is_empty() || results.iter().any(|r| !r.ok) {
        ConnectionStatus::Offline
    } else {
        ConnectionStatus::Online
    }
}

pub fn service_label(url: &str) -> String {
    let lower = url.to_ascii_lowercase();
    if lower.contains("google") {
        "Google Drive".into()
    } else if lower.contains("dropbox") {
        "Dropbox".into()
    } else if lower.contains("onedrive") {
        "OneDrive".into()
    } else if let Some(host) = url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split(['/', '?', '#'])
        .next()
    {
        host.to_string()
    } else {
        url.to_string()
    }
}

pub fn failed_services(results: &[LinkCheck]) -> String {
    results
        .iter()
        .filter(|r| !r.ok)
        .map(|r| service_label(&r.url))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_url_fails() {
        let result = check_link("", 1);
        assert!(!result.ok);
        assert_eq!(result.detail, "empty URL");
    }

    #[test]
    fn summarize_counts() {
        let results = vec![
            LinkCheck {
                url: "a".into(),
                ok: true,
                detail: "HTTP 200".into(),
            },
            LinkCheck {
                url: "b".into(),
                ok: false,
                detail: "timeout".into(),
            },
        ];
        assert_eq!(summarize(&results), "1/2 reachable");
    }

    #[test]
    fn invalid_scheme_fails() {
        let result = check_link("not-a-url", 1);
        assert!(!result.ok);
    }

    #[test]
    fn status_offline_when_any_fail() {
        let results = vec![
            LinkCheck {
                url: "https://www.google.com".into(),
                ok: true,
                detail: "HTTP 200".into(),
            },
            LinkCheck {
                url: "https://www.dropbox.com".into(),
                ok: false,
                detail: "timeout".into(),
            },
        ];
        assert_eq!(status_from_results(&results), ConnectionStatus::Offline);
        assert_eq!(failed_services(&results), "Dropbox");
        assert_eq!(status_from_results(&[]), ConnectionStatus::Offline);
    }

    #[test]
    fn service_labels_known_hosts() {
        assert_eq!(
            service_label("https://www.google.com/generate_204"),
            "Google Drive"
        );
        assert_eq!(service_label("https://example.com/path"), "example.com");
    }
}
