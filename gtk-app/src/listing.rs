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

/// Angular `PathService.parseLocation`. GTK maps `/` and `local` to the local remote.
pub fn parse_location(raw_input: &str, known_remotes: &[String]) -> Option<(String, String)> {
    if raw_input.is_empty() {
        return None;
    }
    let mut normalized = raw_input.replace('\\', "/");
    if normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    if normalized.eq_ignore_ascii_case("starred") || normalized.eq_ignore_ascii_case("starred:") {
        return Some(("starred".into(), String::new()));
    }

    for remote in known_remotes {
        if !is_drive_remote(remote) {
            continue;
        }
        let r_norm = remote.replace('\\', "/");
        let r_low = r_norm.to_lowercase();
        let in_low = normalized.to_lowercase();
        let prefix_ok = in_low == r_low
            || in_low.starts_with(&format!("{r_low}/"))
            || in_low.starts_with(&format!("{r_low}:"));
        if !prefix_ok {
            continue;
        }
        let rest = if normalized.len() >= r_norm.len() {
            normalized[r_norm.len()..]
                .trim_start_matches(['/', ':'])
                .to_string()
        } else {
            String::new()
        };
        return Some(drive_location(remote, &rest));
    }

    // An absolute path is local, even when a directory name contains a colon —
    // `/home/ada/2024:notes` is a folder, not the remote `/home/ada/2024`.
    // rclone remote names cannot start with `/`, so this can never shadow one.
    if normalized.starts_with('/') {
        return Some((
            "local".into(),
            if normalized == "/" {
                "/".into()
            } else {
                normalized
            },
        ));
    }

    if let Some(colon) = normalized.find(':') {
        if colon > 0 {
            let r_name = &normalized[..colon];
            if r_name != "/" && !r_name.is_empty() {
                let r_path = normalized[colon + 1..].trim_start_matches('/').to_string();
                return Some((r_name.to_string(), r_path));
            }
        }
    }

    if known_remotes
        .iter()
        .any(|r| r == &normalized || r == raw_input)
    {
        return Some((normalized, String::new()));
    }

    if known_remotes.iter().any(|r| r == "/") {
        let clean = normalized.trim_start_matches('/').to_string();
        return Some((
            "local".into(),
            if clean.is_empty() {
                "/".into()
            } else {
                format!("/{clean}")
            },
        ));
    }

    None
}

fn is_drive_remote(name: &str) -> bool {
    name == "/"
        || name.eq_ignore_ascii_case("local")
        || name.starts_with('/')
        || looks_like_windows_root(name)
}

fn looks_like_windows_root(name: &str) -> bool {
    let n = name.replace('\\', "/");
    let bytes = n.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn drive_location(remote: &str, rest: &str) -> (String, String) {
    if remote == "/" || remote.eq_ignore_ascii_case("local") {
        let path = if rest.is_empty() {
            "/".into()
        } else if rest.starts_with('/') {
            rest.to_string()
        } else {
            format!("/{rest}")
        };
        return ("local".into(), path);
    }
    if remote.starts_with('/') {
        let path = if rest.is_empty() {
            remote.to_string()
        } else {
            format!("{}/{rest}", remote.trim_end_matches('/'))
        };
        return ("local".into(), path);
    }
    (remote.to_string(), rest.to_string())
}

/// Path bar: Angular `parseLocation`, then relative append (`navigateToPath` fallback).
pub fn resolve_path_bar(
    raw_input: &str,
    known_remotes: &[String],
    current_remote: &str,
    current_path: &str,
) -> (String, String) {
    let trimmed = raw_input.trim();
    if trimmed.is_empty() {
        return (current_remote.to_string(), current_path.to_string());
    }
    if let Some(parsed) = parse_location(trimmed, known_remotes) {
        return parsed;
    }
    let normalized = trimmed.replace('\\', "/");
    let normalized = normalized.trim_matches('/').to_string();
    if current_remote == "starred" || current_remote.is_empty() {
        return ("local".into(), normalized);
    }
    let path = if current_path.is_empty() || current_path == "/" {
        // `normalized` had its slashes trimmed above, so it can never start
        // with one — the second `current_remote == "local"` arm this used to
        // carry was unreachable.
        if current_remote == "local" {
            format!("/{normalized}")
        } else {
            normalized
        }
    } else {
        format!("{}/{normalized}", current_path.trim_end_matches('/'))
    };
    (current_remote.to_string(), path)
}

/// Parent folder + leaf name when `path` is a file (Angular `pendingPreviewFilePath`).
pub fn split_file_nav(path: &str, is_file: bool) -> Option<(String, String)> {
    if !is_file || path.is_empty() || path.ends_with('/') || path.ends_with('\\') {
        return None;
    }
    let trimmed = path.trim_end_matches(['/', '\\']);
    let name = trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|n| !n.is_empty())?
        .to_string();
    Some((parent_remote_path(trimmed), name))
}

/// `operations/stat` when present; otherwise Angular-style file-name heuristic.
pub fn classify_nav_file(stat_is_dir: Option<bool>, remote: &str, path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    match stat_is_dir {
        Some(is_dir) => !is_dir,
        None => {
            let spec = if remote == "local" {
                path.to_string()
            } else {
                format!("{remote}:{path}")
            };
            crate::jobs::source_looks_like_file(&spec)
        }
    }
}

/// Starred (and stale generation) results must not replace the collection list.
/// Angular `starredMode` cancels in-flight `listReadGroups` before painting stars.
pub fn should_apply_directory_list(starred: bool, current_gen: u64, job_gen: u64) -> bool {
    !starred && current_gen == job_gen
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

    #[test]
    fn directory_list_results_do_not_overwrite_starred() {
        assert!(should_apply_directory_list(false, 3, 3));
        assert!(!should_apply_directory_list(true, 3, 3));
        assert!(!should_apply_directory_list(false, 4, 3));
        assert!(!should_apply_directory_list(true, 4, 3));
    }

    #[test]
    fn parse_location_matches_angular_rules() {
        let remotes = ["testdrive".into(), "local".into(), "guilocal".into()];
        assert_eq!(
            parse_location("testdrive:Photos/README.md", &remotes),
            Some(("testdrive".into(), "Photos/README.md".into()))
        );
        assert_eq!(
            parse_location("testdrive", &remotes),
            Some(("testdrive".into(), String::new()))
        );
        assert_eq!(
            parse_location("/tmp/foo", &remotes),
            Some(("local".into(), "/tmp/foo".into()))
        );
        assert_eq!(
            parse_location("local:/tmp/bar", &remotes),
            Some(("local".into(), "/tmp/bar".into()))
        );
        assert_eq!(
            parse_location("starred", &remotes),
            Some(("starred".into(), String::new()))
        );
        assert_eq!(parse_location("Photos", &remotes), None);
        assert_eq!(
            parse_location("Photos", &["testdrive".into(), "/".into()]),
            Some(("local".into(), "/Photos".into()))
        );
        assert_eq!(
            parse_location("/home/me/docs", &["/home/me".into(), "testdrive".into()]),
            Some(("local".into(), "/home/me/docs".into()))
        );
        assert_eq!(
            parse_location("C:\\Users\\ada", &["C:".into()]),
            Some(("C:".into(), "Users/ada".into()))
        );
    }

    #[test]
    fn parse_location_treats_absolute_paths_with_colons_as_local() {
        let remotes = vec!["/".to_string(), "drive".to_string()];
        // A folder name may legally contain ':'. Splitting on the first colon
        // used to send the user to a remote named "/home/ada/2024".
        assert_eq!(
            parse_location("/home/ada/2024:notes/receipts", &remotes),
            Some(("local".into(), "/home/ada/2024:notes/receipts".into()))
        );
        assert_eq!(
            parse_location("/home/ada/notes:2024", &remotes),
            Some(("local".into(), "/home/ada/notes:2024".into()))
        );
        // Real remotes still win.
        assert_eq!(
            parse_location("drive:Photos", &remotes),
            Some(("drive".into(), "Photos".into()))
        );
        // A relative remote-looking path is still parsed as a remote.
        assert_eq!(
            parse_location("otherremote:sub/dir", &remotes),
            Some(("otherremote".into(), "sub/dir".into()))
        );
    }

    #[test]
    fn resolve_path_bar_appends_relative_segments() {
        let remotes = ["testdrive".into(), "local".into()];
        assert_eq!(
            resolve_path_bar("Photos", &remotes, "testdrive", ""),
            ("testdrive".into(), "Photos".into())
        );
        assert_eq!(
            resolve_path_bar("README.md", &remotes, "testdrive", "Photos"),
            ("testdrive".into(), "Photos/README.md".into())
        );
        assert_eq!(
            resolve_path_bar("testdrive:Docs", &remotes, "testdrive", "Photos"),
            ("testdrive".into(), "Docs".into())
        );
        assert_eq!(
            resolve_path_bar("", &remotes, "testdrive", "Photos"),
            ("testdrive".into(), "Photos".into())
        );
    }

    #[test]
    fn split_file_nav_and_classify_file() {
        assert_eq!(
            split_file_nav("Photos/README.md", true),
            Some(("Photos".into(), "README.md".into()))
        );
        assert_eq!(
            split_file_nav("README.md", true),
            Some((String::new(), "README.md".into()))
        );
        assert_eq!(split_file_nav("Photos", false), None);
        assert_eq!(split_file_nav("Photos/", true), None);
        assert!(classify_nav_file(
            Some(false),
            "testdrive",
            "Photos/README.md"
        ));
        assert!(!classify_nav_file(Some(true), "testdrive", "Photos"));
        assert!(classify_nav_file(None, "testdrive", "Photos/README.md"));
        assert!(!classify_nav_file(None, "testdrive", "Photos"));
        assert!(!classify_nav_file(None, "testdrive", ""));
    }
}
