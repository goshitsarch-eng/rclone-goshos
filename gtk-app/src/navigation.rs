//! Cross-view navigation targets, matching the Angular `NavigationDispatcherService`.

use crate::automation::AutomationRecord;
use crate::operations::AppTab;
use crate::store::JobInfo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavTarget {
    Dashboard { tab: AppTab, remote: Option<String> },
    Files { remote: String, path: String },
    Flow { quick_run: Option<String> },
    Job { id: u64 },
    Serve { id: String },
    Automation { id: String },
    Updates,
    Alerts,
}

impl NavTarget {
    pub fn for_job(job: &JobInfo) -> Self {
        if job.origin == "quickrun" || job.origin == "flow" {
            return Self::Flow {
                quick_run: nonempty(&job.profile),
            };
        }
        let tab = match job.operation.as_str() {
            "mount" => AppTab::Mount,
            "serve" => AppTab::Serve,
            _ => AppTab::Operations,
        };
        Self::Dashboard {
            tab,
            remote: nonempty(&job.remote),
        }
    }

    pub fn for_serve(fs: &str, profile: Option<&str>) -> Self {
        let (remote, _) = crate::rclone::split_remote_path(fs);
        if !remote.is_empty() && remote != "local" {
            return Self::Dashboard {
                tab: AppTab::Serve,
                remote: Some(remote),
            };
        }
        Self::Flow {
            quick_run: profile.filter(|s| !s.is_empty()).map(|s| s.to_string()),
        }
    }

    pub fn for_automation(record: &AutomationRecord) -> Self {
        if let Some(id) = record.id.strip_prefix("quick:") {
            return Self::Flow {
                quick_run: nonempty(id),
            };
        }
        Self::Dashboard {
            tab: match record.operation {
                crate::operations::OperationType::Mount => AppTab::Mount,
                crate::operations::OperationType::Serve => AppTab::Serve,
                _ => AppTab::Operations,
            },
            remote: nonempty(&record.remote),
        }
    }
}

/// Angular `openFromBrowseQueryParam` / `PathNavigationService.parseLocation`.
/// Accepts `--browse remote[:path]`, `--browse-path`, and nautilus URLs.
pub fn parse_browse_args(args: &[String]) -> Option<(String, String)> {
    let mut browse = None;
    let mut path = None;
    for (idx, arg) in args.iter().enumerate() {
        match arg.as_str() {
            "--browse" => browse = args.get(idx + 1).cloned(),
            "--browse-path" => path = args.get(idx + 1).cloned(),
            other if idx > 0 && !other.starts_with("--") => {
                if let Some(parsed) = parse_browse_url(other) {
                    return Some(parsed);
                }
            }
            _ => {}
        }
    }
    let browse = browse?;
    if browse.contains(':') && path.is_none() {
        return Some(crate::rclone::split_remote_path(&browse));
    }
    Some((browse, path.unwrap_or_default()))
}

pub fn parse_browse_url(input: &str) -> Option<(String, String)> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(query) = query_string(trimmed) {
        if let Some(remote) = query_param(&query, "browse") {
            return Some((remote, query_param(&query, "path").unwrap_or_default()));
        }
    }
    if let Some(idx) = trimmed.find("#/nautilus") {
        return parse_nautilus_segments(&trimmed[idx + "#/nautilus".len()..]);
    }
    if let Some(idx) = trimmed.find("/nautilus") {
        return parse_nautilus_segments(&trimmed[idx + "/nautilus".len()..]);
    }
    None
}

fn query_string(input: &str) -> Option<String> {
    let after_q = input.split_once('?')?.1;
    Some(after_q.split(['#']).next().unwrap_or(after_q).to_string())
}

fn query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=')?;
        if name == key {
            return Some(
                urlencoding::decode(value)
                    .unwrap_or(std::borrow::Cow::Borrowed(value))
                    .into_owned(),
            );
        }
    }
    None
}

fn parse_nautilus_segments(rest: &str) -> Option<(String, String)> {
    let rest = rest
        .split(['?', '#'])
        .next()
        .unwrap_or(rest)
        .trim_start_matches('/');
    if rest.is_empty() {
        return None;
    }
    let mut parts = rest.split('/');
    let remote = decode_segment(parts.next()?);
    if remote.is_empty() {
        return None;
    }
    let path = parts.map(decode_segment).collect::<Vec<_>>().join("/");
    Some((remote, path))
}

fn decode_segment(value: &str) -> String {
    urlencoding::decode(value)
        .unwrap_or(std::borrow::Cow::Borrowed(value))
        .into_owned()
}

fn nonempty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::OperationType;
    use chrono::Utc;
    use serde_json::json;

    fn job(origin: &str, operation: &str, remote: &str, profile: &str) -> JobInfo {
        JobInfo {
            id: 7,
            operation: operation.into(),
            remote: remote.into(),
            profile: profile.into(),
            status: "running".into(),
            origin: origin.into(),
            start_time: Utc::now(),
            error: None,
            dry_run: false,
            src: String::new(),
            dst: String::new(),
            group: "job/7".into(),
            stats: json!({}),
            transferring: json!([]),
            duration: 0.0,
            progress: 0.0,
            output: json!({}),
            completed: json!([]),
            parent_job_id: None,
        }
    }

    #[test]
    fn job_from_dashboard_opens_operations_tab() {
        assert_eq!(
            NavTarget::for_job(&job("dashboard", "copy", "drive", "default")),
            NavTarget::Dashboard {
                tab: AppTab::Operations,
                remote: Some("drive".into()),
            }
        );
        assert_eq!(
            NavTarget::for_job(&job("dashboard", "mount", "drive", "")),
            NavTarget::Dashboard {
                tab: AppTab::Mount,
                remote: Some("drive".into()),
            }
        );
    }

    #[test]
    fn job_from_flow_selects_quick_run() {
        assert_eq!(
            NavTarget::for_job(&job("quickrun", "sync", "drive", "nightly")),
            NavTarget::Flow {
                quick_run: Some("nightly".into()),
            }
        );
    }

    #[test]
    fn serve_uses_fs_remote() {
        assert_eq!(
            NavTarget::for_serve("photos:public", None),
            NavTarget::Dashboard {
                tab: AppTab::Serve,
                remote: Some("photos".into()),
            }
        );
        assert_eq!(
            NavTarget::for_serve("/home/user", Some("local-http")),
            NavTarget::Flow {
                quick_run: Some("local-http".into()),
            }
        );
    }

    #[test]
    fn automation_splits_quick_and_remote() {
        let remote = AutomationRecord {
            id: "remote:drive:sync:nightly".into(),
            name: "drive nightly".into(),
            remote: "drive".into(),
            profile: "nightly".into(),
            operation: OperationType::Sync,
            cron: String::new(),
            cron_enabled: true,
            watch_enabled: false,
            watch_delay: 0,
            watch_changed_only: false,
            sources: vec![],
            next_run: None,
            last_run: None,
        };
        assert_eq!(
            NavTarget::for_automation(&remote),
            NavTarget::Dashboard {
                tab: AppTab::Operations,
                remote: Some("drive".into()),
            }
        );
        let mut quick = remote.clone();
        quick.id = "quick:abc-123".into();
        assert_eq!(
            NavTarget::for_automation(&quick),
            NavTarget::Flow {
                quick_run: Some("abc-123".into()),
            }
        );
    }

    #[test]
    fn parses_browse_cli_and_urls() {
        assert_eq!(
            parse_browse_args(&["app".into(), "--browse".into(), "drive:Photos".into()]),
            Some(("drive".into(), "Photos".into()))
        );
        assert_eq!(
            parse_browse_args(&[
                "app".into(),
                "--browse".into(),
                "drive".into(),
                "--browse-path".into(),
                "Inbox".into()
            ]),
            Some(("drive".into(), "Inbox".into()))
        );
        assert_eq!(
            parse_browse_url("https://app.local/?browse=gdrive&path=Photos%2F2024"),
            Some(("gdrive".into(), "Photos/2024".into()))
        );
        assert_eq!(
            parse_browse_url("#/nautilus/gdrive/Photos/2024"),
            Some(("gdrive".into(), "Photos/2024".into()))
        );
        assert_eq!(
            parse_browse_url("/nautilus/gdrive/Docs"),
            Some(("gdrive".into(), "Docs".into()))
        );
        assert_eq!(
            parse_browse_args(&["app".into(), "/nautilus/local/home/ada".into()]),
            Some(("local".into(), "home/ada".into()))
        );
        assert_eq!(parse_browse_url("https://app.local/dashboard"), None);
    }
}
