//! Cancellable `operations/list` jobs — Angular `listReadGroups` / `getRemotePaths`.

use crate::rclone::{parent_remote_path, remote_fs, DirEntry, RcClient, RcError};
use serde_json::{json, Value};

pub const LIST_GROUP_LEFT_PREFIX: &str = "ui/nautilus/list-left-";
pub const LIST_GROUP_RIGHT_PREFIX: &str = "ui/nautilus/list-right-";

#[derive(Debug, Clone)]
pub enum ListStart {
    Job(u64),
    Ready(Vec<DirEntry>),
}

#[derive(Debug, Clone)]
pub enum ListJobState {
    Running,
    Finished(Vec<DirEntry>),
    Failed(String),
    Cancelled,
}

pub fn list_read_group(primary: bool) -> String {
    let token = uuid::Uuid::new_v4().as_simple().to_string();
    list_read_group_with_token(primary, &token[..8])
}

pub fn list_read_group_with_token(primary: bool, token: &str) -> String {
    if primary {
        format!("{LIST_GROUP_LEFT_PREFIX}{token}")
    } else {
        format!("{LIST_GROUP_RIGHT_PREFIX}{token}")
    }
}

pub fn list_dir_payload(fs: &str, remote: &str, group: &str) -> Value {
    json!({
        "fs": fs,
        "remote": remote,
        "_async": true,
        "_group": group,
    })
}

pub fn list_target(remote: &str, path: &str) -> (String, String) {
    let fs = if remote == "local" {
        "/".to_string()
    } else {
        remote_fs(remote, "")
    };
    let remote_path = if remote == "local" {
        path.trim_start_matches('/').to_string()
    } else {
        path.to_string()
    };
    (fs, remote_path)
}

pub fn parent_list_target(remote: &str, path: &str) -> (String, String) {
    list_target(remote, &parent_remote_path(path))
}

/// Record the location we are leaving so Back can return here (Angular `newHistory`).
pub fn push_nav_history(
    history: &mut Vec<(String, String)>,
    future: &mut Vec<(String, String)>,
    remote: &str,
    path: &str,
) {
    history.push((remote.to_string(), path.to_string()));
    future.clear();
}

pub fn same_nav_location(a_remote: &str, a_path: &str, b_remote: &str, b_path: &str) -> bool {
    a_remote == b_remote && a_path.trim_matches('/') == b_path.trim_matches('/')
}

pub fn pop_nav_back(
    history: &mut Vec<(String, String)>,
    future: &mut Vec<(String, String)>,
    current_remote: &str,
    current_path: &str,
) -> Option<(String, String)> {
    let prev = history.pop()?;
    future.push((current_remote.to_string(), current_path.to_string()));
    Some(prev)
}

pub fn pop_nav_forward(
    history: &mut Vec<(String, String)>,
    future: &mut Vec<(String, String)>,
    current_remote: &str,
    current_path: &str,
) -> Option<(String, String)> {
    let next = future.pop()?;
    history.push((current_remote.to_string(), current_path.to_string()));
    Some(next)
}

pub fn listing_siblings(entries: &[DirEntry]) -> Vec<(String, bool)> {
    entries
        .iter()
        .map(|entry| (entry.name.clone(), entry.is_dir))
        .collect()
}

pub fn parent_listing_siblings(client: &RcClient, remote: &str, path: &str) -> Vec<(String, bool)> {
    let (fs, list_path) = parent_list_target(remote, path);
    client
        .list_dir(&fs, &list_path)
        .ok()
        .map(|entries| listing_siblings(&entries))
        .unwrap_or_default()
}

pub fn resolve_siblings(
    client: Option<&RcClient>,
    remote: &str,
    path: &str,
    siblings: &[(String, bool)],
) -> Vec<(String, bool)> {
    if !siblings.is_empty() {
        return siblings.to_vec();
    }
    client
        .map(|c| parent_listing_siblings(c, remote, path))
        .unwrap_or_default()
}

pub fn is_cancelled_error(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("cancelled")
        || lower.contains("canceled")
        || lower.contains("abort")
        || lower.contains("stopped by user")
        || lower.contains("job stopped")
}

pub fn entries_from_list_value(value: &Value) -> Vec<DirEntry> {
    let list = value
        .get("list")
        .or_else(|| value.get("output").and_then(|output| output.get("list")))
        .and_then(|item| item.as_array())
        .cloned()
        .unwrap_or_default();
    list.iter().filter_map(DirEntry::from_value).collect()
}

pub fn parse_list_job_status(status: &Value) -> ListJobState {
    let finished = status
        .get("finished")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let success = status
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let error = status
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if is_cancelled_error(&error) {
        return ListJobState::Cancelled;
    }
    if !finished {
        return ListJobState::Running;
    }
    if !success {
        if error.is_empty() {
            return ListJobState::Cancelled;
        }
        return ListJobState::Failed(error);
    }
    let output = status.get("output").unwrap_or(status);
    ListJobState::Finished(entries_from_list_value(output))
}

pub fn start_list_dir(
    client: &RcClient,
    fs: &str,
    remote: &str,
    group: &str,
) -> Result<ListStart, RcError> {
    let value = client.call("operations/list", list_dir_payload(fs, remote, group))?;
    if let Some(jobid) = value.get("jobid").and_then(|v| v.as_u64()) {
        return Ok(ListStart::Job(jobid));
    }
    Ok(ListStart::Ready(entries_from_list_value(&value)))
}

pub fn poll_list_job(client: &RcClient, jobid: u64) -> ListJobState {
    match client.job_status(jobid) {
        Ok(status) => parse_list_job_status(&status),
        Err(err) => {
            if is_cancelled_error(&err.to_string()) {
                ListJobState::Cancelled
            } else {
                ListJobState::Failed(err.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool) -> Value {
        json!({
            "Name": name,
            "Path": name,
            "IsDir": is_dir,
            "Size": 4,
            "MimeType": "text/plain",
            "ModTime": "2026-01-01T00:00:00Z",
        })
    }

    #[test]
    fn list_groups_match_angular_prefixes() {
        assert_eq!(
            list_read_group_with_token(true, "abcd1234"),
            "ui/nautilus/list-left-abcd1234"
        );
        assert_eq!(
            list_read_group_with_token(false, "efgh5678"),
            "ui/nautilus/list-right-efgh5678"
        );
        let live = list_read_group(true);
        assert!(live.starts_with(LIST_GROUP_LEFT_PREFIX));
        assert_eq!(live.len(), LIST_GROUP_LEFT_PREFIX.len() + 8);
    }

    #[test]
    fn list_payload_is_async_and_grouped() {
        let payload = list_dir_payload("testdrive:", "Photos", "ui/nautilus/list-left-ab12cd34");
        assert_eq!(payload["fs"], "testdrive:");
        assert_eq!(payload["remote"], "Photos");
        assert_eq!(payload["_async"], true);
        assert_eq!(payload["_group"], "ui/nautilus/list-left-ab12cd34");
    }

    #[test]
    fn list_target_uses_local_root_and_remote_fs() {
        assert_eq!(
            list_target("local", "/tmp/rclone-test-remote/Photos"),
            ("/".into(), "tmp/rclone-test-remote/Photos".into())
        );
        assert_eq!(
            list_target("testdrive", "Photos"),
            ("testdrive:".into(), "Photos".into())
        );
        assert_eq!(
            parent_list_target("testdrive", "Photos/README.md"),
            ("testdrive:".into(), "Photos".into())
        );
        assert_eq!(
            parent_list_target("local", "/tmp/rclone-test-remote/Photos/README.md"),
            ("/".into(), "tmp/rclone-test-remote/Photos".into())
        );
    }

    #[test]
    fn parse_list_states_cover_running_finished_failed_cancelled() {
        assert!(matches!(
            parse_list_job_status(&json!({ "finished": false, "success": false })),
            ListJobState::Running
        ));
        let finished = parse_list_job_status(&json!({
            "finished": true,
            "success": true,
            "output": { "list": [entry("README.md", false), entry("Photos", true)] }
        }));
        match finished {
            ListJobState::Finished(entries) => {
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].name, "README.md");
                assert!(!entries[0].is_dir);
                assert!(entries[1].is_dir);
            }
            other => panic!("expected finished list, got {other:?}"),
        }
        match parse_list_job_status(&json!({
            "finished": true,
            "success": false,
            "error": "directory not found"
        })) {
            ListJobState::Failed(err) => assert_eq!(err, "directory not found"),
            other => panic!("expected failed, got {other:?}"),
        }
        assert!(matches!(
            parse_list_job_status(&json!({
                "finished": true,
                "success": false,
                "error": "job was aborted by user"
            })),
            ListJobState::Cancelled
        ));
        assert!(matches!(
            parse_list_job_status(&json!({
                "finished": false,
                "success": false,
                "error": "Operation cancelled"
            })),
            ListJobState::Cancelled
        ));
        assert!(matches!(
            parse_list_job_status(&json!({
                "finished": true,
                "success": false,
                "error": ""
            })),
            ListJobState::Cancelled
        ));
    }

    #[test]
    fn cancelled_error_matches_angular_wording() {
        assert!(is_cancelled_error("Operation cancelled"));
        assert!(is_cancelled_error("Canceled by caller"));
        assert!(is_cancelled_error("request aborted"));
        assert!(is_cancelled_error("job stopped by user"));
        assert!(!is_cancelled_error("directory not found"));
        assert!(!is_cancelled_error(""));
    }

    #[test]
    fn entries_read_list_from_output_or_root() {
        let from_output = entries_from_list_value(&json!({
            "output": { "list": [entry("ok.txt", false)] }
        }));
        assert_eq!(from_output[0].name, "ok.txt");
        let from_root = entries_from_list_value(&json!({
            "list": [entry("bad.txt", false)]
        }));
        assert_eq!(from_root[0].name, "bad.txt");
        assert!(entries_from_list_value(&json!({})).is_empty());
        assert!(entries_from_list_value(&json!({ "list": [] })).is_empty());
    }

    #[test]
    fn resolve_siblings_keeps_provided_and_falls_back_empty() {
        let given = vec![("README.md".into(), false)];
        assert_eq!(
            resolve_siblings(None, "testdrive", "Photos/README.md", &given),
            given
        );
        assert!(resolve_siblings(None, "testdrive", "Photos/README.md", &[]).is_empty());
    }

    #[test]
    fn nav_history_records_back_and_clears_forward() {
        let mut history = vec![("testdrive".into(), String::new())];
        let mut future = vec![("testdrive".into(), "old".into())];
        push_nav_history(&mut history, &mut future, "testdrive", "Photos");
        assert_eq!(
            history,
            vec![
                ("testdrive".into(), String::new()),
                ("testdrive".into(), "Photos".into())
            ]
        );
        assert!(future.is_empty());
        assert!(same_nav_location(
            "testdrive",
            "Photos",
            "testdrive",
            "Photos/"
        ));
        assert!(same_nav_location(
            "testdrive",
            "Photos",
            "testdrive",
            "Photos"
        ));
        assert!(!same_nav_location("testdrive", "Photos", "testdrive", ""));
        let back = pop_nav_back(&mut history, &mut future, "testdrive", "verify");
        assert_eq!(back, Some(("testdrive".into(), "Photos".into())));
        assert_eq!(future, vec![("testdrive".into(), "verify".into())]);
        let fwd = pop_nav_forward(&mut history, &mut future, "testdrive", "Photos");
        assert_eq!(fwd, Some(("testdrive".into(), "verify".into())));
    }

    #[test]
    fn listing_siblings_preserve_dir_flag() {
        let entries = vec![
            DirEntry {
                name: "Photos".into(),
                is_dir: true,
                ..DirEntry::default()
            },
            DirEntry {
                name: "README.md".into(),
                is_dir: false,
                ..DirEntry::default()
            },
        ];
        assert_eq!(
            listing_siblings(&entries),
            vec![("Photos".into(), true), ("README.md".into(), false)]
        );
    }
}
