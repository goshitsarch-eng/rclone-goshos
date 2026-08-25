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
}
