//! Streaming upload handlers for the web server.

use std::path::{Path, PathBuf};

use axum::{
    extract::{Multipart, State},
    response::Json,
};
use futures::StreamExt;

use crate::rclone::commands::upload::{UploadBatchParams, execute_upload_batch};
use crate::server::state::{ApiResponse, AppError, WebServerState};
use crate::utils::types::origin::Origin;

struct BatchMeta {
    id: String,
    file_index: usize,
    total_files: usize,
}

pub async fn stream_upload_handler(
    State(state): State<WebServerState>,
    mut multipart: Multipart,
) -> Result<Json<ApiResponse<String>>, AppError> {
    let (mut remote, mut path) = (String::new(), String::new());
    let (mut origin, mut job_id) = (None, None);
    let (mut raw_batch_id, mut raw_file_index, mut raw_total_files) = (None, None, None);
    let mut raw_mtime: Option<i64> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(anyhow::Error::msg(e.to_string())))?
    {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "remote" => remote = field.text().await.unwrap_or_default(),
            "path" => path = field.text().await.unwrap_or_default(),
            "origin" => origin = serde_json::from_str(&field.text().await.unwrap_or_default()).ok(),
            "batchId" => raw_batch_id = Some(field.text().await.unwrap_or_default()),
            "jobId" => job_id = field.text().await.unwrap_or_default().parse().ok(),
            "fileIndex" => raw_file_index = field.text().await.unwrap_or_default().parse().ok(),
            "totalFiles" => raw_total_files = field.text().await.unwrap_or_default().parse().ok(),
            "mtime" => raw_mtime = field.text().await.unwrap_or_default().parse().ok(),
            "file" => {
                let filename = field.file_name().unwrap_or("unnamed").replace('\\', "/");
                let batch = build_batch_meta(raw_batch_id, raw_file_index, raw_total_files);

                let temp_dir = std::env::temp_dir();
                let (batch_dir, temp_path) = resolve_temp_path(&temp_dir, &filename, &batch).await;

                write_field_to_file(field, &temp_path, raw_mtime).await?;

                if let Some(ref b) = batch {
                    return finalize_batch_upload(
                        &state, b, batch_dir, remote, path, origin, job_id,
                    )
                    .await;
                }

                let params = build_upload_params(
                    remote,
                    path,
                    vec![temp_path.to_string_lossy().to_string()],
                    origin,
                    Some(temp_path),
                    job_id,
                );
                let res = run_upload(&state, params).await?;
                return Ok(Json(ApiResponse::success(res)));
            }
            _ => {}
        }
    }
    Err(AppError::BadRequest(anyhow::Error::msg("No file found")))
}

fn build_batch_meta(
    raw_batch_id: Option<String>,
    raw_file_index: Option<usize>,
    raw_total_files: Option<usize>,
) -> Option<BatchMeta> {
    match (raw_batch_id, raw_file_index, raw_total_files) {
        (Some(id), Some(file_index), Some(total_files)) => Some(BatchMeta {
            id,
            file_index,
            total_files,
        }),
        _ => None,
    }
}

/// Reduce a multipart `filename` to a relative path that cannot escape the
/// batch directory.
///
/// Folder uploads legitimately carry a relative path (`photos/2024/a.jpg`), so
/// interior directories are kept — but the value comes straight from an
/// attacker-controllable `Content-Disposition` header, and `Path::join` with an
/// absolute path *replaces* the base entirely. Anything that is not a plain
/// name component (root, `..`, a Windows drive prefix) is dropped, and the
/// whole value is rejected if nothing usable is left.
fn safe_relative_path(filename: &str) -> Option<PathBuf> {
    use std::path::Component;

    let mut out = PathBuf::new();
    for component in Path::new(filename).components() {
        match component {
            Component::Normal(part) => {
                if part.is_empty() {
                    continue;
                }
                out.push(part);
            }
            // RootDir / Prefix would reset the join; ParentDir would walk out.
            Component::RootDir | Component::Prefix(_) | Component::ParentDir => continue,
            Component::CurDir => continue,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

async fn resolve_temp_path(
    temp_dir: &Path,
    filename: &str,
    batch: &Option<BatchMeta>,
) -> (Option<PathBuf>, PathBuf) {
    if let Some(b) = batch {
        // The batch id also reaches the filesystem, so give it the same
        // treatment and fall back to a generated name if it is unusable.
        let batch_id = safe_relative_path(&b.id)
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
        let dir = temp_dir.join(format!("rclone_batch_{batch_id}"));
        tokio::fs::create_dir_all(&dir).await.ok();

        let relative = safe_relative_path(filename)
            .unwrap_or_else(|| PathBuf::from(format!("upload_{}", uuid::Uuid::new_v4().simple())));
        if let Some(parent) = relative.parent().filter(|p| !p.as_os_str().is_empty()) {
            tokio::fs::create_dir_all(dir.join(parent)).await.ok();
        }
        let target = dir.join(&relative);
        (Some(dir.clone()), target)
    } else {
        (
            None,
            temp_dir.join(format!("upload_{}.tmp", uuid::Uuid::new_v4())),
        )
    }
}

async fn write_field_to_file(
    field: axum::extract::multipart::Field<'_>,
    temp_path: &Path,
    mtime: Option<i64>,
) -> Result<(), AppError> {
    let mut file = tokio::fs::File::create(temp_path)
        .await
        .map_err(|e| AppError::InternalServerError(anyhow::Error::msg(e)))?;
    let mut stream = field;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| AppError::BadRequest(anyhow::Error::msg(e)))?;
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|e| AppError::InternalServerError(anyhow::Error::msg(e)))?;
    }
    drop(file);

    if let Some(mtime_ms) = mtime {
        let system_time = std::time::UNIX_EPOCH + std::time::Duration::from_millis(mtime_ms as u64);
        let times = std::fs::FileTimes::new().set_modified(system_time);
        if let Ok(file) = std::fs::File::options().write(true).open(temp_path) {
            let _ = file.set_times(times);
        }
    }

    Ok(())
}

async fn finalize_batch_upload(
    state: &WebServerState,
    batch: &BatchMeta,
    batch_dir: Option<PathBuf>,
    remote: String,
    path: String,
    origin: Option<Origin>,
    job_id: Option<u64>,
) -> Result<Json<ApiResponse<String>>, AppError> {
    if batch.file_index < batch.total_files - 1 {
        return Ok(Json(ApiResponse::success("File buffered".into())));
    }
    let bdir = batch_dir.expect("batch_dir present in batch mode");
    let mut entries = Vec::new();
    let mut reader = tokio::fs::read_dir(&bdir)
        .await
        .map_err(|e| AppError::InternalServerError(anyhow::Error::msg(e)))?;
    while let Some(entry) = reader.next_entry().await.ok().flatten() {
        entries.push(entry.path().to_string_lossy().to_string());
    }

    let params = build_upload_params(remote, path, entries, origin, Some(bdir), job_id);
    let res = run_upload(state, params).await?;
    Ok(Json(ApiResponse::success(res)))
}

fn build_upload_params(
    remote: String,
    path: String,
    local_paths: Vec<String>,
    origin: Option<Origin>,
    cleanup_dir: Option<PathBuf>,
    existing_jobid: Option<u64>,
) -> UploadBatchParams {
    UploadBatchParams {
        remote,
        path,
        local_paths,
        origin,
        group: None,
        cleanup_dir,
        existing_jobid,
        no_cache: false,
    }
}

async fn run_upload(state: &WebServerState, params: UploadBatchParams) -> Result<String, AppError> {
    execute_upload_batch(state.app_handle.clone(), params)
        .await
        .map_err(|e| AppError::InternalServerError(anyhow::Error::msg(e)))
}

#[cfg(test)]
mod tests {
    use super::safe_relative_path;
    use std::path::{Path, PathBuf};

    #[test]
    fn keeps_legitimate_folder_upload_paths() {
        assert_eq!(
            safe_relative_path("photos/2024/a.jpg"),
            Some(PathBuf::from("photos/2024/a.jpg"))
        );
        assert_eq!(
            safe_relative_path("report.pdf"),
            Some(PathBuf::from("report.pdf"))
        );
        assert_eq!(
            safe_relative_path("./trip/./b.png"),
            Some(PathBuf::from("trip/b.png"))
        );
    }

    #[test]
    fn strips_absolute_paths_so_join_cannot_be_reset() {
        // `Path::join` with an absolute path discards the base entirely, so an
        // absolute Content-Disposition filename would write anywhere on disk.
        let base = Path::new("/tmp/rclone_batch_x");
        assert_eq!(
            base.join("/data/rclone-bin/rclone"),
            Path::new("/data/rclone-bin/rclone")
        );

        let safe = safe_relative_path("/data/rclone-bin/rclone").unwrap();
        assert_eq!(safe, PathBuf::from("data/rclone-bin/rclone"));
        assert!(base.join(&safe).starts_with(base));
    }

    #[test]
    fn strips_parent_traversal() {
        let base = Path::new("/tmp/rclone_batch_x");
        let safe = safe_relative_path("../../../etc/cron.d/pwn").unwrap();
        assert_eq!(safe, PathBuf::from("etc/cron.d/pwn"));
        assert!(base.join(&safe).starts_with(base));

        let mixed = safe_relative_path("ok/../../escape.txt").unwrap();
        assert_eq!(mixed, PathBuf::from("ok/escape.txt"));
        assert!(base.join(&mixed).starts_with(base));
    }

    #[test]
    fn rejects_names_with_nothing_usable_left() {
        assert_eq!(safe_relative_path(""), None);
        assert_eq!(safe_relative_path("/"), None);
        assert_eq!(safe_relative_path("../.."), None);
        assert_eq!(safe_relative_path("."), None);
    }
}
