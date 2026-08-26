//! Cross-view navigation targets, matching the Angular `NavigationDispatcherService`.

use crate::automation::AutomationRecord;
use crate::operations::AppTab;
use crate::store::JobInfo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavTarget {
    Dashboard {
        tab: AppTab,
        remote: Option<String>,
    },
    Files {
        remote: String,
        path: String,
    },
    Flow {
        quick_run: Option<String>,
    },
    Job {
        id: u64,
    },
    Serve {
        id: String,
    },
    Automation {
        id: String,
    },
    Updates,
    Alerts,
    Preferences {
        page: Option<String>,
    },
    RemoteConfig {
        remote: String,
        step: Option<String>,
        profile: Option<String>,
    },
    Onboarding,
    About,
    Logs,
    Shortcuts,
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
    let trimmed = strip_app_scheme(input.trim());
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
    if let Some(rest) = trimmed.strip_prefix("nautilus/") {
        return parse_nautilus_segments(rest);
    }
    if trimmed == "nautilus" {
        return Some(("local".into(), String::new()));
    }
    None
}

/// Tauri `rclone-manager://` deep-link scheme (also registered on the desktop file).
fn strip_app_scheme(input: &str) -> &str {
    input
        .strip_prefix("rclone-manager://")
        .or_else(|| input.strip_prefix("rclone-manager:"))
        .unwrap_or(input)
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

/// Startup navigation from CLI flags or Angular-style hash/path URLs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    pub target: NavTarget,
    pub standalone: bool,
}

pub fn parse_launch_args(args: &[String], standalone_dialogs: bool) -> Option<LaunchRequest> {
    let kind = standalone_kind(args);
    let mut tab = AppTab::General;
    let mut remote = None;
    let mut target = None;
    let mut config_step = None;
    let mut config_profile = None;
    for (idx, arg) in args.iter().enumerate() {
        let value = args.get(idx + 1).cloned();
        match arg.as_str() {
            "--dashboard" => {
                if let Some(next) = value.as_deref().filter(|v| !v.starts_with('-')) {
                    tab = AppTab::parse(next).unwrap_or(AppTab::General);
                }
                target = Some(NavTarget::Dashboard {
                    tab,
                    remote: remote.clone(),
                });
            }
            "--tab" => {
                if let Some(next) = value.as_deref().filter(|v| !v.starts_with('-')) {
                    tab = AppTab::parse(next).unwrap_or(tab);
                }
                target = Some(NavTarget::Dashboard {
                    tab,
                    remote: remote.clone(),
                });
            }
            "--remote" => {
                remote = value.filter(|v| !v.is_empty() && !v.starts_with('-'));
                target = Some(NavTarget::Dashboard {
                    tab,
                    remote: remote.clone(),
                });
            }
            "--flow" => {
                target = Some(NavTarget::Flow {
                    quick_run: value.filter(|v| !v.starts_with('-')),
                });
            }
            "--quick-run" => {
                target = Some(NavTarget::Flow {
                    quick_run: value.filter(|v| !v.starts_with('-')),
                });
            }
            "--job" => {
                if let Some(id) = value.and_then(|v| v.parse().ok()) {
                    target = Some(NavTarget::Job { id });
                }
            }
            "--serve" => {
                if let Some(id) = value.filter(|v| !v.is_empty() && !v.starts_with('-')) {
                    target = Some(NavTarget::Serve { id });
                }
            }
            "--automation" => {
                if let Some(id) = value.filter(|v| !v.is_empty() && !v.starts_with('-')) {
                    target = Some(NavTarget::Automation { id });
                }
            }
            "--updates" => target = Some(NavTarget::Updates),
            "--alerts" => target = Some(NavTarget::Alerts),
            "--preferences" | "--settings" => {
                target = Some(NavTarget::Preferences {
                    page: value
                        .filter(|v| !v.is_empty() && !v.starts_with('-'))
                        .and_then(|page| normalize_prefs_page(&page)),
                });
            }
            "--onboarding" => target = Some(NavTarget::Onboarding),
            "--about" => target = Some(NavTarget::About),
            "--logs" => target = Some(NavTarget::Logs),
            "--shortcuts" => target = Some(NavTarget::Shortcuts),
            "--remote-config" => {
                if let Some(name) = value.filter(|v| !v.is_empty() && !v.starts_with('-')) {
                    target = Some(NavTarget::RemoteConfig {
                        remote: name,
                        step: None,
                        profile: None,
                    });
                }
            }
            "--step" => {
                config_step = value.filter(|v| !v.is_empty() && !v.starts_with('-'));
            }
            "--profile" => {
                config_profile = value.filter(|v| !v.is_empty() && !v.starts_with('-'));
            }
            other if idx > 0 && !other.starts_with("--") => {
                if let Some(parsed) = parse_route_url(other) {
                    return Some(LaunchRequest {
                        standalone: is_standalone_target(
                            &parsed,
                            other,
                            kind.as_deref(),
                            standalone_dialogs,
                        ),
                        target: parsed,
                    });
                }
            }
            _ => {}
        }
    }
    if let Some(NavTarget::RemoteConfig { step, profile, .. }) = target.as_mut() {
        if step.is_none() {
            *step = config_step;
        }
        if profile.is_none() {
            *profile = config_profile;
        }
    }
    if target.is_none() {
        if let Some((remote, path)) = parse_browse_args(args) {
            target = Some(NavTarget::Files { remote, path });
        }
    }
    if target.is_none() {
        target = match kind.as_deref() {
            Some("flow") => Some(NavTarget::Flow { quick_run: None }),
            Some("main") => Some(NavTarget::Dashboard {
                tab,
                remote: remote.clone(),
            }),
            Some("nautilus") | Some("true") | Some("") => Some(NavTarget::Files {
                remote: "local".into(),
                path: String::new(),
            }),
            _ => None,
        };
    }
    let target = target?;
    let standalone = is_standalone_target(&target, "", kind.as_deref(), standalone_dialogs);
    Some(LaunchRequest { target, standalone })
}

pub fn parse_route_url(input: &str) -> Option<NavTarget> {
    let input = strip_app_scheme(input);
    if let Some((remote, path)) = parse_browse_url(input) {
        return Some(NavTarget::Files { remote, path });
    }
    let path = route_path(input)?;
    let mut parts = path.split('/');
    match parts.next()? {
        "dashboard" | "main" => {
            let tab = parts
                .next()
                .and_then(AppTab::parse)
                .unwrap_or(AppTab::General);
            let remote = parts.next().map(decode_segment).and_then(|s| nonempty(&s));
            Some(NavTarget::Dashboard { tab, remote })
        }
        "flow" => Some(NavTarget::Flow {
            quick_run: parts.next().map(decode_segment).and_then(|s| nonempty(&s)),
        }),
        "job" => parts
            .next()
            .and_then(|id| id.parse().ok())
            .map(|id| NavTarget::Job { id }),
        "serve" => parts
            .next()
            .map(decode_segment)
            .and_then(|id| nonempty(&id))
            .map(|id| NavTarget::Serve { id }),
        "automation" => parts
            .next()
            .map(decode_segment)
            .and_then(|id| nonempty(&id))
            .map(|id| NavTarget::Automation { id }),
        "updates" => Some(NavTarget::Updates),
        "alerts" => Some(NavTarget::Alerts),
        "preferences" | "settings" => Some(NavTarget::Preferences {
            page: parts
                .next()
                .map(decode_segment)
                .and_then(|page| normalize_prefs_page(&page)),
        }),
        "remote-config" | "remoteconfig" => {
            let remote = parts
                .next()
                .map(decode_segment)
                .and_then(|name| nonempty(&name))?;
            Some(NavTarget::RemoteConfig {
                remote,
                step: parts
                    .next()
                    .map(decode_segment)
                    .and_then(|step| nonempty(&step)),
                profile: parts
                    .next()
                    .map(decode_segment)
                    .and_then(|profile| nonempty(&profile)),
            })
        }
        "onboarding" => Some(NavTarget::Onboarding),
        "about" => Some(NavTarget::About),
        "logs" => Some(NavTarget::Logs),
        "shortcuts" | "keyboard-shortcuts" => Some(NavTarget::Shortcuts),
        _ => None,
    }
}

pub fn normalize_prefs_page(page: &str) -> Option<String> {
    match page {
        "general" | "appearance" => Some("general".into()),
        "core" | "engine" => Some("core".into()),
        "security" => Some("security".into()),
        "developer" | "dev" => Some("developer".into()),
        other => nonempty(other),
    }
}

fn route_path(input: &str) -> Option<String> {
    let trimmed = strip_app_scheme(input.trim());
    if let Some((_, hash)) = trimmed.split_once('#') {
        let raw = hash
            .split('?')
            .next()
            .unwrap_or(hash)
            .trim_start_matches('/');
        if !raw.is_empty() {
            return Some(raw.to_string());
        }
    }
    let without_query = trimmed.split('?').next().unwrap_or(trimmed);
    let path = if let Some(rest) = without_query
        .strip_prefix("https://")
        .or_else(|| without_query.strip_prefix("http://"))
    {
        rest.split_once('/')?.1
    } else {
        without_query.trim_start_matches('/')
    };
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        return None;
    }
    Some(path.to_string())
}

fn standalone_kind(args: &[String]) -> Option<String> {
    for arg in args {
        if arg == "--standalone" {
            return Some("nautilus".into());
        }
        if let Some(value) = arg.strip_prefix("--standalone=") {
            return Some(value.to_string());
        }
    }
    None
}

fn standalone_query(input: &str) -> Option<String> {
    query_string(input).and_then(|q| query_param(&q, "standalone"))
}

fn is_standalone_target(
    target: &NavTarget,
    url: &str,
    kind: Option<&str>,
    standalone_dialogs: bool,
) -> bool {
    let query = standalone_query(url);
    match target {
        NavTarget::Files { .. } => true,
        NavTarget::Flow { .. } => {
            kind == Some("flow")
                || query.as_deref() == Some("flow")
                || standalone_dialogs && url.contains("#/flow")
        }
        NavTarget::Dashboard { .. } => kind == Some("main") || query.as_deref() == Some("main"),
        _ => false,
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
            destinations: vec![],
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
        assert_eq!(
            parse_browse_url("rclone-manager://nautilus/gdrive/Photos"),
            Some(("gdrive".into(), "Photos".into()))
        );
        assert_eq!(
            parse_browse_url("rclone-manager:///nautilus/local/home"),
            Some(("local".into(), "home".into()))
        );
    }

    #[test]
    fn parses_dashboard_flow_and_job_urls() {
        assert_eq!(
            parse_route_url("#/dashboard/mount/testdrive"),
            Some(NavTarget::Dashboard {
                tab: AppTab::Mount,
                remote: Some("testdrive".into()),
            })
        );
        assert_eq!(
            parse_route_url("https://app.local/#/flow/nightly"),
            Some(NavTarget::Flow {
                quick_run: Some("nightly".into()),
            })
        );
        assert_eq!(
            parse_route_url("https://app.local/dashboard/operations/drive"),
            Some(NavTarget::Dashboard {
                tab: AppTab::Operations,
                remote: Some("drive".into()),
            })
        );
        assert_eq!(parse_route_url("#/job/42"), Some(NavTarget::Job { id: 42 }));
        assert_eq!(
            parse_route_url("#/serve/http-1"),
            Some(NavTarget::Serve {
                id: "http-1".into()
            })
        );
        assert_eq!(
            parse_route_url("/automation/quick%3Anightly"),
            Some(NavTarget::Automation {
                id: "quick:nightly".into(),
            })
        );
        assert_eq!(
            parse_route_url("#/main"),
            Some(NavTarget::Dashboard {
                tab: AppTab::General,
                remote: None,
            })
        );
        assert_eq!(
            parse_route_url("#/main/operations/testdrive"),
            Some(NavTarget::Dashboard {
                tab: AppTab::Operations,
                remote: Some("testdrive".into()),
            })
        );
        assert_eq!(parse_route_url("#/updates"), Some(NavTarget::Updates));
        assert_eq!(parse_route_url("#/alerts"), Some(NavTarget::Alerts));
        assert_eq!(
            parse_route_url("#/preferences/developer"),
            Some(NavTarget::Preferences {
                page: Some("developer".into()),
            })
        );
        assert_eq!(
            parse_route_url("#/settings/core"),
            Some(NavTarget::Preferences {
                page: Some("core".into()),
            })
        );
        assert_eq!(
            parse_route_url("#/remote-config/testdrive/sync/nightly"),
            Some(NavTarget::RemoteConfig {
                remote: "testdrive".into(),
                step: Some("sync".into()),
                profile: Some("nightly".into()),
            })
        );
        assert_eq!(parse_route_url("#/onboarding"), Some(NavTarget::Onboarding));
        assert_eq!(parse_route_url("#/about"), Some(NavTarget::About));
        assert_eq!(parse_route_url("#/logs"), Some(NavTarget::Logs));
        assert_eq!(
            parse_route_url("#/keyboard-shortcuts"),
            Some(NavTarget::Shortcuts)
        );
        assert_eq!(
            parse_route_url("rclone-manager://dashboard/mount/testdrive"),
            Some(NavTarget::Dashboard {
                tab: AppTab::Mount,
                remote: Some("testdrive".into()),
            })
        );
        assert_eq!(
            parse_route_url("rclone-manager://#/preferences/developer"),
            Some(NavTarget::Preferences {
                page: Some("developer".into()),
            })
        );
        assert_eq!(
            parse_route_url("rclone-manager://flow/nightly"),
            Some(NavTarget::Flow {
                quick_run: Some("nightly".into()),
            })
        );
        assert_eq!(
            parse_route_url("#/nautilus/gdrive/Photos"),
            Some(NavTarget::Files {
                remote: "gdrive".into(),
                path: "Photos".into(),
            })
        );
    }

    #[test]
    fn launch_args_open_standalone_files() {
        let browse = parse_launch_args(
            &["app".into(), "--browse".into(), "drive:Photos".into()],
            false,
        );
        assert_eq!(
            browse,
            Some(LaunchRequest {
                target: NavTarget::Files {
                    remote: "drive".into(),
                    path: "Photos".into(),
                },
                standalone: true,
            })
        );
        let nautilus =
            parse_launch_args(&["app".into(), "#/nautilus/local/home/ada".into()], false);
        assert_eq!(
            nautilus,
            Some(LaunchRequest {
                target: NavTarget::Files {
                    remote: "local".into(),
                    path: "home/ada".into(),
                },
                standalone: true,
            })
        );
        let flag = parse_launch_args(&["app".into(), "--standalone".into()], false);
        assert_eq!(
            flag,
            Some(LaunchRequest {
                target: NavTarget::Files {
                    remote: "local".into(),
                    path: String::new(),
                },
                standalone: true,
            })
        );
    }

    #[test]
    fn launch_args_deep_link_main_window() {
        let dash = parse_launch_args(
            &[
                "app".into(),
                "--dashboard".into(),
                "mount".into(),
                "--remote".into(),
                "testdrive".into(),
            ],
            false,
        );
        assert_eq!(
            dash,
            Some(LaunchRequest {
                target: NavTarget::Dashboard {
                    tab: AppTab::Mount,
                    remote: Some("testdrive".into()),
                },
                standalone: false,
            })
        );
        let remote_only = parse_launch_args(
            &[
                "app".into(),
                "--tab".into(),
                "operations".into(),
                "--remote".into(),
                "testdrive".into(),
            ],
            false,
        );
        assert_eq!(
            remote_only,
            Some(LaunchRequest {
                target: NavTarget::Dashboard {
                    tab: AppTab::Operations,
                    remote: Some("testdrive".into()),
                },
                standalone: false,
            })
        );
        let flow = parse_launch_args(&["app".into(), "--flow".into()], false);
        assert_eq!(
            flow,
            Some(LaunchRequest {
                target: NavTarget::Flow { quick_run: None },
                standalone: false,
            })
        );
        let flow_qr = parse_launch_args(&["app".into(), "--flow".into(), "nightly".into()], false);
        assert_eq!(
            flow_qr,
            Some(LaunchRequest {
                target: NavTarget::Flow {
                    quick_run: Some("nightly".into()),
                },
                standalone: false,
            })
        );
        let standalone_flow = parse_launch_args(
            &[
                "app".into(),
                "--standalone=flow".into(),
                "--quick-run".into(),
                "abc".into(),
            ],
            false,
        );
        assert_eq!(
            standalone_flow,
            Some(LaunchRequest {
                target: NavTarget::Flow {
                    quick_run: Some("abc".into()),
                },
                standalone: true,
            })
        );
        let job = parse_launch_args(&["app".into(), "--job".into(), "9".into()], false);
        assert_eq!(
            job,
            Some(LaunchRequest {
                target: NavTarget::Job { id: 9 },
                standalone: false,
            })
        );
        let updates = parse_launch_args(&["app".into(), "--updates".into()], false);
        assert_eq!(
            updates,
            Some(LaunchRequest {
                target: NavTarget::Updates,
                standalone: false,
            })
        );
        let prefs = parse_launch_args(&["app".into(), "--preferences".into(), "dev".into()], false);
        assert_eq!(
            prefs,
            Some(LaunchRequest {
                target: NavTarget::Preferences {
                    page: Some("developer".into()),
                },
                standalone: false,
            })
        );
        let scheme = parse_launch_args(
            &[
                "app".into(),
                "rclone-manager://dashboard/mount/testdrive".into(),
            ],
            false,
        );
        assert_eq!(
            scheme,
            Some(LaunchRequest {
                target: NavTarget::Dashboard {
                    tab: AppTab::Mount,
                    remote: Some("testdrive".into()),
                },
                standalone: false,
            })
        );
        let config = parse_launch_args(
            &[
                "app".into(),
                "--remote-config".into(),
                "testdrive".into(),
                "--step".into(),
                "sync".into(),
                "--profile".into(),
                "nightly".into(),
            ],
            false,
        );
        assert_eq!(
            config,
            Some(LaunchRequest {
                target: NavTarget::RemoteConfig {
                    remote: "testdrive".into(),
                    step: Some("sync".into()),
                    profile: Some("nightly".into()),
                },
                standalone: false,
            })
        );
        assert_eq!(
            parse_launch_args(&["app".into(), "--onboarding".into()], false),
            Some(LaunchRequest {
                target: NavTarget::Onboarding,
                standalone: false,
            })
        );
        assert_eq!(
            parse_launch_args(&["app".into(), "--about".into()], false),
            Some(LaunchRequest {
                target: NavTarget::About,
                standalone: false,
            })
        );
    }
}
