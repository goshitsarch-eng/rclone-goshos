//! Plan StatusNotifier tray menus from runtime state (testable, no GTK).

use crate::jobs::{find_active_quick_run, preferred_mount_profile_name, profile_is_active};
use crate::operations::OperationType;
use crate::rclone::{MountedRemote, ServeItem};
use crate::store::{AppStore, JobInfo, RemoteInfo, RemoteMeta};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    ShowWindow,
    OpenFiles,
    Quit,
    UnmountAll,
    StopJobs,
    StopServes,
    Status,
    MountRemote {
        remote: String,
        profile: String,
    },
    UnmountRemote {
        remote: String,
        profile: String,
    },
    BrowseRemote {
        remote: String,
        profile: String,
    },
    BrowseInApp(String),
    StartProfile {
        remote: String,
        op: String,
        profile: String,
    },
    StopProfile {
        remote: String,
        op: String,
        profile: String,
    },
    StartQuickRun(String),
    StopQuickRun(String),
}

/// CLI token for `--tray-action` (Windows NotifyIcon / macOS NSStatusItem helpers).
pub fn encode_tray_action(action: &TrayAction) -> String {
    match action {
        TrayAction::ShowWindow => "show-window".into(),
        TrayAction::OpenFiles => "open-files".into(),
        TrayAction::Quit => "quit".into(),
        TrayAction::UnmountAll => "unmount-all".into(),
        TrayAction::StopJobs => "stop-jobs".into(),
        TrayAction::StopServes => "stop-serves".into(),
        TrayAction::Status => "status".into(),
        TrayAction::MountRemote { remote, profile } => {
            format!("mount|{remote}|{profile}")
        }
        TrayAction::UnmountRemote { remote, profile } => {
            format!("unmount|{remote}|{profile}")
        }
        TrayAction::BrowseRemote { remote, profile } => {
            if profile.is_empty() {
                format!("browse|{remote}")
            } else {
                format!("browse|{remote}|{profile}")
            }
        }
        TrayAction::BrowseInApp(remote) => format!("browse-in|{remote}"),
        TrayAction::StartProfile {
            remote,
            op,
            profile,
        } => format!("start|{remote}|{op}|{profile}"),
        TrayAction::StopProfile {
            remote,
            op,
            profile,
        } => format!("stop|{remote}|{op}|{profile}"),
        TrayAction::StartQuickRun(id) => format!("qr-start|{id}"),
        TrayAction::StopQuickRun(id) => format!("qr-stop|{id}"),
    }
}

pub fn parse_tray_action(token: &str) -> Option<TrayAction> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    let mut parts = token.split('|');
    let kind = parts.next()?;
    match kind {
        "show-window" => Some(TrayAction::ShowWindow),
        "open-files" => Some(TrayAction::OpenFiles),
        "quit" => Some(TrayAction::Quit),
        "unmount-all" => Some(TrayAction::UnmountAll),
        "stop-jobs" => Some(TrayAction::StopJobs),
        "stop-serves" => Some(TrayAction::StopServes),
        "status" => Some(TrayAction::Status),
        "mount" => Some(TrayAction::MountRemote {
            remote: parts.next()?.into(),
            profile: parts.next()?.into(),
        }),
        "unmount" => Some(TrayAction::UnmountRemote {
            remote: parts.next()?.into(),
            profile: parts.next()?.into(),
        }),
        "browse" => Some(TrayAction::BrowseRemote {
            remote: parts.next()?.into(),
            profile: parts.next().unwrap_or("").into(),
        }),
        "browse-in" => Some(TrayAction::BrowseInApp(parts.next()?.into())),
        "start" => Some(TrayAction::StartProfile {
            remote: parts.next()?.into(),
            op: parts.next()?.into(),
            profile: parts.next()?.into(),
        }),
        "stop" => Some(TrayAction::StopProfile {
            remote: parts.next()?.into(),
            op: parts.next()?.into(),
            profile: parts.next()?.into(),
        }),
        "qr-start" => Some(TrayAction::StartQuickRun(parts.next()?.into())),
        "qr-stop" => Some(TrayAction::StopQuickRun(parts.next()?.into())),
        _ => None,
    }
}

/// Flatten a nested StatusNotifier menu into `(label, action-token)` pairs.
/// Nested submenu names collapse to the top-level prefix (`drive: Mount · home`).
/// Separators are `("", None)`. Disabled / status-only rows are omitted.
pub fn flatten_tray_menu(menu: &[TrayMenuItem]) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    flatten_tray_menu_into(menu, "", &mut out);
    trim_tray_separators(&mut out);
    out
}

fn flatten_tray_menu_into(
    menu: &[TrayMenuItem],
    prefix: &str,
    out: &mut Vec<(String, Option<String>)>,
) {
    for item in menu {
        if !item.children.is_empty() {
            let next = if prefix.is_empty() {
                item.label.trim_start_matches('●').trim().to_string()
            } else {
                prefix.to_string()
            };
            flatten_tray_menu_into(&item.children, &next, out);
            continue;
        }
        if item.action.is_none() {
            if out.last().is_some_and(|(_, action)| action.is_none()) {
                continue;
            }
            out.push((String::new(), None));
            continue;
        }
        if !item.enabled || matches!(item.action, Some(TrayAction::Status)) {
            continue;
        }
        let Some(action) = item.action.as_ref() else {
            continue;
        };
        let label = if prefix.is_empty() {
            item.label.clone()
        } else {
            format!("{prefix}: {}", item.label)
        };
        out.push((label, Some(encode_tray_action(action))));
    }
}

fn trim_tray_separators(items: &mut Vec<(String, Option<String>)>) {
    while items.first().is_some_and(|(_, action)| action.is_none()) {
        items.remove(0);
    }
    while items.last().is_some_and(|(_, action)| action.is_none()) {
        items.pop();
    }
}

/// Stable encoding used to decide whether a Win/macOS tray helper must restart.
pub fn tray_helper_signature(items: &[(String, Option<String>)]) -> String {
    items
        .iter()
        .map(|(label, action)| match action {
            None => "|".to_string(),
            Some(token) => format!("{label}={token}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn tray_helper_needs_restart(prev: &str, items: &[(String, Option<String>)]) -> bool {
    tray_helper_signature(items) != prev
}

/// Nested menu tree for Windows NotifyIcon / macOS NSStatusItem helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperMenuNode {
    Action {
        label: String,
        token: String,
    },
    Separator,
    Submenu {
        label: String,
        children: Vec<HelperMenuNode>,
    },
}

/// Preserve `plan_tray` nesting (skip disabled / status-only rows).
pub fn helper_menu_nodes(menu: &[TrayMenuItem]) -> Vec<HelperMenuNode> {
    let mut out = Vec::new();
    for item in menu {
        if !item.children.is_empty() {
            let children = helper_menu_nodes(&item.children);
            if !children.is_empty() {
                out.push(HelperMenuNode::Submenu {
                    label: item.label.clone(),
                    children,
                });
            }
            continue;
        }
        if item.action.is_none() {
            if matches!(out.last(), Some(HelperMenuNode::Separator)) {
                continue;
            }
            out.push(HelperMenuNode::Separator);
            continue;
        }
        if !item.enabled || matches!(item.action, Some(TrayAction::Status)) {
            continue;
        }
        if let Some(action) = &item.action {
            out.push(HelperMenuNode::Action {
                label: item.label.clone(),
                token: encode_tray_action(action),
            });
        }
    }
    while matches!(out.first(), Some(HelperMenuNode::Separator)) {
        out.remove(0);
    }
    while matches!(out.last(), Some(HelperMenuNode::Separator)) {
        out.pop();
    }
    out
}

pub fn parse_tray_action_args(args: &[String]) -> Option<TrayAction> {
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if let Some(value) = arg.strip_prefix("--tray-action=") {
            return parse_tray_action(value);
        }
        if arg == "--tray-action" {
            return args.get(i + 1).and_then(|v| parse_tray_action(v));
        }
        i += 1;
    }
    None
}

/// ARGB32 (network byte order) pixmap so StatusNotifier hosts can draw the
/// tray icon when the panel theme has no `folder-remote` name.
pub fn status_icon_argb(size: i32, busy: bool) -> Vec<u8> {
    let size = size.clamp(16, 64) as usize;
    let mut data = vec![0u8; size * size * 4];
    let (r, g, b) = if busy {
        (0xF5u8, 0x7Cu8, 0x00u8)
    } else {
        (0x35u8, 0x84u8, 0xE4u8)
    };
    let cx = (size as i32 - 1) as f32 / 2.0;
    let cy = cx;
    let outer = size as f32 * 0.42;
    let inner = size as f32 * 0.18;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let alpha = if dist <= inner {
                0u8
            } else if dist <= outer {
                255
            } else if dist <= outer + 1.2 {
                ((1.0 - (dist - outer) / 1.2) * 255.0) as u8
            } else {
                0
            };
            let i = (y * size + x) * 4;
            data[i] = alpha;
            data[i + 1] = r;
            data[i + 2] = g;
            data[i + 3] = b;
        }
    }
    data
}

/// Localized Start/Stop label for a tray Quick Run (do not parse English prefixes).
pub fn quick_run_action_label(
    start: bool,
    name: &str,
    start_word: &str,
    stop_word: &str,
) -> String {
    format!("{} {name}", if start { start_word } else { stop_word })
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TrayCaption {
    #[default]
    Literal,
    Jobs {
        active: usize,
        total: usize,
    },
    OpCount {
        op: String,
        active: usize,
        total: usize,
    },
    QuickRuns {
        active: usize,
        total: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayMenuItem {
    pub label: String,
    pub enabled: bool,
    pub action: Option<TrayAction>,
    pub children: Vec<TrayMenuItem>,
    pub caption: TrayCaption,
}

impl TrayMenuItem {
    pub fn action(label: impl Into<String>, action: TrayAction) -> Self {
        Self {
            label: label.into(),
            enabled: true,
            action: Some(action),
            children: Vec::new(),
            caption: TrayCaption::Literal,
        }
    }

    pub fn disabled(label: impl Into<String>, action: TrayAction) -> Self {
        Self {
            label: label.into(),
            enabled: false,
            action: Some(action),
            children: Vec::new(),
            caption: TrayCaption::Literal,
        }
    }

    pub fn submenu(label: impl Into<String>, children: Vec<Self>) -> Self {
        Self {
            label: label.into(),
            enabled: true,
            action: None,
            children,
            caption: TrayCaption::Literal,
        }
    }

    pub fn separator() -> Self {
        Self {
            label: String::new(),
            enabled: true,
            action: None,
            children: Vec::new(),
            caption: TrayCaption::Literal,
        }
    }

    pub fn with_caption(mut self, caption: TrayCaption) -> Self {
        self.caption = caption;
        self
    }
}

pub fn plan_tray(
    remotes: &[RemoteInfo],
    store: &AppStore,
    jobs: &[JobInfo],
    mounts: &[MountedRemote],
    serves: &[ServeItem],
    max_items: usize,
) -> Vec<TrayMenuItem> {
    let mut items = vec![
        TrayMenuItem::action("Show Window", TrayAction::ShowWindow),
        TrayMenuItem::action("Open Files", TrayAction::OpenFiles),
    ];
    let quick: Vec<_> = store
        .quick_runs
        .iter()
        .filter(|q| q.show_on_tray)
        .cloned()
        .collect();
    if !quick.is_empty() {
        items.push(TrayMenuItem::separator());
        let active = quick
            .iter()
            .filter(|qr| find_active_quick_run(jobs, qr).is_some())
            .count();
        let mut children = Vec::new();
        for qr in &quick {
            let running = find_active_quick_run(jobs, qr).is_some();
            if running {
                children.push(TrayMenuItem::action(
                    format!("Stop {}", qr.name),
                    TrayAction::StopQuickRun(qr.id.clone()),
                ));
            } else {
                children.push(TrayMenuItem::action(
                    format!("Run {}", qr.name),
                    TrayAction::StartQuickRun(qr.id.clone()),
                ));
            }
        }
        items.push(TrayMenuItem::submenu("Quick Runs", children).with_caption(
            TrayCaption::QuickRuns {
                active,
                total: quick.len(),
            },
        ));
    }
    let visible: Vec<&RemoteInfo> = remotes
        .iter()
        .filter(|r| {
            store
                .remotes
                .get(&r.name)
                .map(|m| m.show_on_tray)
                .unwrap_or(true)
        })
        .take(max_items.max(1))
        .collect();
    if !visible.is_empty() {
        items.push(TrayMenuItem::separator());
        for remote in visible {
            items.push(remote_submenu(remote, store, jobs, mounts, serves));
        }
        items.push(TrayMenuItem::separator());
        items.push(TrayMenuItem::action("Unmount All", TrayAction::UnmountAll));
        items.push(TrayMenuItem::action("Stop All Jobs", TrayAction::StopJobs));
        items.push(TrayMenuItem::action(
            "Stop All Serves",
            TrayAction::StopServes,
        ));
    }
    items.push(TrayMenuItem::separator());
    items.push(TrayMenuItem::action("Quit", TrayAction::Quit));
    items
}

fn remote_submenu(
    remote: &RemoteInfo,
    store: &AppStore,
    jobs: &[JobInfo],
    mounts: &[MountedRemote],
    serves: &[ServeItem],
) -> TrayMenuItem {
    let name = remote.name.clone();
    let meta = store.remotes.get(&name);
    let mounted = remote.mounted;
    let ops = meta
        .map(RemoteMeta::visible_operations)
        .unwrap_or_else(|| OperationType::ALL.to_vec());
    let active_jobs = jobs
        .iter()
        .filter(|job| job.remote == name && (job.status == "running" || job.status == "starting"))
        .count();
    let total_profiles: usize = ops
        .iter()
        .map(|op| {
            let count = meta.map(|m| m.profile_names(*op).len()).unwrap_or(0);
            if *op == OperationType::Mount {
                count.max(1)
            } else {
                count
            }
        })
        .sum();

    let mut children = vec![TrayMenuItem::disabled(
        if total_profiles == 0 {
            "Jobs [—]".into()
        } else {
            format!("Jobs [{active_jobs}/{total_profiles}]")
        },
        TrayAction::Status,
    )
    .with_caption(TrayCaption::Jobs {
        active: active_jobs,
        total: total_profiles,
    })];
    children.push(TrayMenuItem::separator());

    for op in ops {
        children.push(op_submenu(remote, meta, op, jobs, mounts, serves));
    }

    children.push(TrayMenuItem::separator());
    if mounted {
        children.extend(browse_menu_items(&name, mounts));
    } else {
        children.push(TrayMenuItem::action(
            "Browse in app",
            TrayAction::BrowseInApp(name.clone()),
        ));
    }

    let serving = !remote_serves_for(&name, serves).is_empty();
    let label = remote_tray_label(&name, mounted, active_jobs, serving);
    TrayMenuItem::submenu(label, children)
}

pub fn remote_belongs(fs: &str, name: &str) -> bool {
    fs == name || fs.starts_with(&format!("{name}:"))
}

pub fn remote_mounts_for<'a>(name: &str, mounts: &'a [MountedRemote]) -> Vec<&'a MountedRemote> {
    mounts
        .iter()
        .filter(|mount| remote_belongs(&mount.fs, name))
        .collect()
}

pub fn remote_serves_for<'a>(name: &str, serves: &'a [ServeItem]) -> Vec<&'a ServeItem> {
    serves
        .iter()
        .filter(|serve| remote_belongs(&serve.fs, name))
        .collect()
}

pub fn remote_tray_label(name: &str, mounted: bool, jobs: usize, serving: bool) -> String {
    let mut extras = Vec::new();
    if jobs > 0 {
        extras.push(jobs.to_string());
    }
    if serving {
        extras.push("serve".into());
    }
    let base = if mounted {
        format!("● {name}")
    } else {
        name.to_string()
    };
    if extras.is_empty() {
        base
    } else {
        format!("{base} · {}", extras.join(" · "))
    }
}

fn browse_key(mount: &MountedRemote) -> String {
    if !mount.profile.is_empty() {
        mount.profile.clone()
    } else {
        mount.mount_point.clone()
    }
}

fn browse_menu_items(name: &str, mounts: &[MountedRemote]) -> Vec<TrayMenuItem> {
    let points = remote_mounts_for(name, mounts);
    if points.len() <= 1 {
        let profile = points
            .first()
            .map(|mount| mount.profile.clone())
            .unwrap_or_default();
        return vec![TrayMenuItem::action(
            "Browse",
            TrayAction::BrowseRemote {
                remote: name.to_string(),
                profile,
            },
        )];
    }
    let children = points
        .iter()
        .map(|mount| {
            let key = browse_key(mount);
            let suffix = if !mount.profile.is_empty() {
                mount.profile.as_str()
            } else {
                mount.mount_point.as_str()
            };
            TrayMenuItem::action(
                format!("Browse · {suffix}"),
                TrayAction::BrowseRemote {
                    remote: name.to_string(),
                    profile: key,
                },
            )
        })
        .collect();
    vec![TrayMenuItem::submenu("Browse", children)]
}

fn op_submenu(
    remote: &RemoteInfo,
    meta: Option<&RemoteMeta>,
    op: OperationType,
    jobs: &[JobInfo],
    mounts: &[MountedRemote],
    serves: &[ServeItem],
) -> TrayMenuItem {
    let name = remote.name.clone();
    let mut profiles = meta.map(|m| m.profile_names(op)).unwrap_or_default();
    if profiles.is_empty() && op == OperationType::Mount {
        profiles.push(preferred_mount_profile_name(meta));
    }
    if profiles.is_empty() {
        profiles.push("default".into());
    }
    let active = profiles
        .iter()
        .filter(|pname| profile_is_active(&name, op, pname, jobs, mounts, serves))
        .count();
    let mut children = Vec::new();
    for pname in &profiles {
        let running = profile_is_active(&name, op, pname, jobs, mounts, serves);
        if op == OperationType::Mount {
            if running {
                children.push(TrayMenuItem::action(
                    format!("Unmount · {pname}"),
                    TrayAction::UnmountRemote {
                        remote: name.clone(),
                        profile: pname.clone(),
                    },
                ));
            } else {
                children.push(TrayMenuItem::action(
                    format!("Mount · {pname}"),
                    TrayAction::MountRemote {
                        remote: name.clone(),
                        profile: pname.clone(),
                    },
                ));
            }
            continue;
        }
        let action = if running {
            TrayAction::StopProfile {
                remote: name.clone(),
                op: op.as_str().into(),
                profile: pname.clone(),
            }
        } else {
            TrayAction::StartProfile {
                remote: name.clone(),
                op: op.as_str().into(),
                profile: pname.clone(),
            }
        };
        let label = if running {
            format!("Stop {} · {pname}", op.api_label())
        } else {
            format!("Start {} · {pname}", op.api_label())
        };
        children.push(TrayMenuItem::action(label, action));
    }
    TrayMenuItem::submenu(
        format!("{} [{active}/{}]", op.api_label(), profiles.len()),
        children,
    )
    .with_caption(TrayCaption::OpCount {
        op: op.as_str().into(),
        active,
        total: profiles.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ProfileConfig, QuickRun, RemoteMeta};
    use serde_json::json;

    fn drive_remote(mounted: bool) -> RemoteInfo {
        RemoteInfo {
            name: "drive".into(),
            r#type: "drive".into(),
            mounted,
            serving: false,
            job_active: mounted,
            hidden: false,
            disk_label: String::new(),
        }
    }

    #[test]
    fn plans_profile_and_quick_run_actions() {
        let remotes = vec![drive_remote(true)];
        let mut store = AppStore::default();
        let mut meta = RemoteMeta {
            show_on_tray: true,
            primary_actions: vec!["mount".into(), "sync".into()],
            ..RemoteMeta::default()
        };
        meta.upsert_profile(
            OperationType::Sync,
            ProfileConfig {
                name: "nightly".into(),
                app: Default::default(),
                rclone: json!({ "srcFs": "drive:a", "dstFs": "/tmp" }),
            },
        );
        meta.upsert_profile(
            OperationType::Mount,
            ProfileConfig {
                name: "home".into(),
                app: Default::default(),
                rclone: json!({ "mountPoint": "/mnt/drive" }),
            },
        );
        store.remotes.insert("drive".into(), meta);
        let mut qr = QuickRun::new("Photos".into(), OperationType::Copy, "drive".into());
        qr.show_on_tray = true;
        qr.last_job_id = Some(9);
        store.quick_runs.push(qr);
        let jobs = vec![JobInfo {
            id: 9,
            operation: "copy".into(),
            remote: "drive".into(),
            profile: "default".into(),
            status: "running".into(),
            origin: "quick-run".into(),
            start_time: chrono::Utc::now(),
            error: None,
            dry_run: false,
            src: "drive:a".into(),
            dst: "/tmp".into(),
            group: "job/9".into(),
            stats: json!({}),
            transferring: json!([]),
            duration: 0.0,
            progress: 0.0,
            output: json!({}),
            completed: json!([]),
            parent_job_id: None,
        }];
        let mounts = vec![MountedRemote::new("drive:", "/mnt/drive")];
        let plan = plan_tray(&remotes, &store, &jobs, &mounts, &[], 8);
        assert_eq!(
            plan.first().and_then(|i| i.action.clone()),
            Some(TrayAction::ShowWindow)
        );
        assert_eq!(
            plan.last().and_then(|i| i.action.clone()),
            Some(TrayAction::Quit)
        );
        assert!(plan
            .iter()
            .any(|i| i.action == Some(TrayAction::UnmountAll)));
        let quit_idx = plan
            .iter()
            .position(|i| i.action == Some(TrayAction::Quit))
            .unwrap();
        let unmount_idx = plan
            .iter()
            .position(|i| i.action == Some(TrayAction::UnmountAll))
            .unwrap();
        assert!(unmount_idx < quit_idx);

        let remote = plan
            .iter()
            .find(|i| i.label.contains("drive"))
            .expect("remote submenu");
        assert!(remote.children.iter().any(|c| matches!(
            c.caption,
            TrayCaption::Jobs { total, .. } if total >= 1
        )));
        let sync = remote
            .children
            .iter()
            .find(|c| matches!(c.caption, TrayCaption::OpCount { ref op, .. } if op == "sync"))
            .expect("sync submenu");
        assert!(sync.children.iter().any(|c| matches!(
            c.action,
            Some(TrayAction::StartProfile { ref op, ref profile, .. })
                if op == "sync" && profile == "nightly"
        )));
        let mount = remote
            .children
            .iter()
            .find(|c| matches!(c.caption, TrayCaption::OpCount { ref op, .. } if op == "mount"))
            .expect("mount submenu");
        assert!(mount.children.iter().any(|c| matches!(
            c.action,
            Some(TrayAction::UnmountRemote { ref profile, .. }) if profile == "home"
        )));
        assert!(
            mount.children.iter().any(|c| matches!(
                c.action,
                Some(TrayAction::MountRemote { ref profile, .. }) if profile == "home"
            )) || mount.children.iter().any(|c| matches!(
                c.action,
                Some(TrayAction::UnmountRemote { ref profile, .. }) if profile == "home"
            ))
        );
        let quick = plan
            .iter()
            .find(|i| matches!(i.caption, TrayCaption::QuickRuns { .. }))
            .unwrap();
        assert_eq!(
            quick.children[0].action,
            Some(TrayAction::StopQuickRun(store.quick_runs[0].id.clone()))
        );
    }

    #[test]
    fn mount_uses_non_default_saved_profile() {
        let remotes = vec![drive_remote(false)];
        let mut store = AppStore::default();
        let mut meta = RemoteMeta {
            show_on_tray: true,
            primary_actions: vec!["mount".into()],
            ..RemoteMeta::default()
        };
        meta.upsert_profile(
            OperationType::Mount,
            ProfileConfig {
                name: "media".into(),
                app: Default::default(),
                rclone: json!({ "mountPoint": "/mnt/media" }),
            },
        );
        store.remotes.insert("drive".into(), meta);
        let plan = plan_tray(&remotes, &store, &[], &[], &[], 4);
        let remote = plan.iter().find(|i| i.label.contains("drive")).unwrap();
        let mount = remote
            .children
            .iter()
            .find(|c| matches!(c.caption, TrayCaption::OpCount { ref op, .. } if op == "mount"))
            .unwrap();
        assert!(mount.children.iter().any(|c| matches!(
            c.action,
            Some(TrayAction::MountRemote { ref profile, .. }) if profile == "media"
        )));
        assert!(!mount.children.iter().any(|c| matches!(
            c.action,
            Some(TrayAction::MountRemote { ref profile, .. }) if profile == "default"
        )));
    }

    #[test]
    fn quick_run_labels_use_translated_verbs() {
        assert_eq!(
            quick_run_action_label(true, "Nightly", "Start", "Stop"),
            "Start Nightly"
        );
        assert_eq!(
            quick_run_action_label(false, "Nightly", "Başlat", "Durdur"),
            "Durdur Nightly"
        );
    }

    #[test]
    fn status_icon_argb_is_network_order_with_visible_pixels() {
        let idle = status_icon_argb(22, false);
        assert_eq!(idle.len(), 22 * 22 * 4);
        assert!(idle.chunks(4).any(|px| px[0] > 200 && px[3] == 0xE4));
        let busy = status_icon_argb(16, true);
        assert_eq!(busy.len(), 16 * 16 * 4);
        assert!(busy.chunks(4).any(|px| px[0] > 200 && px[1] == 0xF5));
    }

    #[test]
    fn tray_action_cli_tokens_roundtrip() {
        let actions = [
            TrayAction::ShowWindow,
            TrayAction::OpenFiles,
            TrayAction::Quit,
            TrayAction::UnmountAll,
            TrayAction::StopJobs,
            TrayAction::StopServes,
            TrayAction::MountRemote {
                remote: "testdrive".into(),
                profile: "default".into(),
            },
            TrayAction::StartQuickRun("qr-1".into()),
            TrayAction::BrowseRemote {
                remote: "testdrive".into(),
                profile: "home".into(),
            },
        ];
        for action in actions {
            let token = encode_tray_action(&action);
            assert_eq!(parse_tray_action(&token), Some(action));
        }
        assert_eq!(
            parse_tray_action_args(&[
                "app".into(),
                "--tray-action".into(),
                "browse|testdrive".into()
            ]),
            Some(TrayAction::BrowseRemote {
                remote: "testdrive".into(),
                profile: String::new(),
            })
        );
        assert_eq!(
            parse_tray_action("browse|testdrive|home"),
            Some(TrayAction::BrowseRemote {
                remote: "testdrive".into(),
                profile: "home".into(),
            })
        );
        assert_eq!(
            encode_tray_action(&TrayAction::BrowseRemote {
                remote: "testdrive".into(),
                profile: "home".into(),
            }),
            "browse|testdrive|home"
        );
        assert_eq!(
            parse_tray_action_args(&["app".into(), "--tray-action=quit".into()]),
            Some(TrayAction::Quit)
        );
        assert_eq!(parse_tray_action(""), None);
        assert_eq!(parse_tray_action("nope"), None);
    }

    #[test]
    fn flatten_tray_menu_includes_prefixed_remote_actions() {
        let remotes = vec![drive_remote(false)];
        let mut store = AppStore::default();
        let mut meta = RemoteMeta {
            show_on_tray: true,
            primary_actions: vec!["mount".into()],
            ..RemoteMeta::default()
        };
        meta.upsert_profile(
            OperationType::Mount,
            ProfileConfig {
                name: "media".into(),
                app: Default::default(),
                rclone: json!({ "mountPoint": "/mnt/media" }),
            },
        );
        store.remotes.insert("drive".into(), meta);
        let mut qr = QuickRun::new("Photos".into(), OperationType::Copy, "drive".into());
        qr.show_on_tray = true;
        store.quick_runs.push(qr.clone());
        let plan = plan_tray(&remotes, &store, &[], &[], &[], 4);
        let flat = flatten_tray_menu(&plan);
        assert!(
            flat.iter().any(|(label, action)| {
                action.as_deref() == Some("mount|drive|media") && label == "drive: Mount · media"
            }),
            "flat menu missing mount: {flat:?}"
        );
        assert!(flat.iter().any(|(label, action)| {
            action.as_deref() == Some("browse-in|drive") && label == "drive: Browse in app"
        }));
        assert!(flat.iter().any(|(label, action)| {
            action.as_deref() == Some(&format!("qr-start|{}", qr.id)) && label.contains("Photos")
        }));
        assert!(!flat
            .iter()
            .any(|(_, action)| action.as_deref() == Some("status")));
        let sig = tray_helper_signature(&flat);
        assert!(!tray_helper_needs_restart(&sig, &flat));
        let mut changed = flat.clone();
        changed.push(("Quit".into(), Some("quit".into())));
        assert!(tray_helper_needs_restart(&sig, &changed));
        let ps = crate::platform::windows_notifyicon_ps1("C:\\app.exe", &flat);
        assert!(ps.contains("mount|drive|media"));
        let swift = crate::platform::macos_status_item_swift("/opt/app", &flat);
        assert!(swift.contains("mount|drive|media"));
        assert!(swift.contains("qr-start|"));
        assert_eq!(remote_tray_label("drive", true, 0, false), "● drive");
        assert_eq!(
            remote_tray_label("drive", true, 2, true),
            "● drive · 2 · serve"
        );
        assert_eq!(remote_tray_label("drive", false, 1, false), "drive · 1");

        let nodes = helper_menu_nodes(&plan);
        assert!(nodes.iter().any(|node| matches!(
            node,
            HelperMenuNode::Submenu { label, children }
                if label.contains("drive")
                    && children.iter().any(|child| match child {
                        HelperMenuNode::Submenu { children, .. } => children.iter().any(|leaf| {
                            matches!(
                                leaf,
                                HelperMenuNode::Action { token, .. }
                                    if token == "mount|drive|media"
                            )
                        }),
                        HelperMenuNode::Action { token, .. } => token == "mount|drive|media",
                        _ => false,
                    })
        )));
        let nested_ps = crate::platform::windows_notifyicon_ps1_nodes("C:\\app.exe", &nodes);
        assert!(nested_ps.contains("DropDownItems"));
        assert!(nested_ps.contains("mount|drive|media"));
        let nested_swift = crate::platform::macos_status_item_swift_nodes("/opt/app", &nodes);
        assert!(nested_swift.contains(".submenu"));
        assert!(nested_swift.contains("mount|drive|media"));
        assert!(nested_swift.contains("qr-start|"));
    }

    #[test]
    fn browse_submenu_lists_each_mount_profile() {
        let remotes = vec![drive_remote(true)];
        let mut store = AppStore::default();
        let mut meta = RemoteMeta {
            show_on_tray: true,
            primary_actions: vec!["mount".into()],
            ..RemoteMeta::default()
        };
        meta.upsert_profile(
            OperationType::Mount,
            ProfileConfig {
                name: "home".into(),
                app: Default::default(),
                rclone: json!({ "mountPoint": "/mnt/home" }),
            },
        );
        meta.upsert_profile(
            OperationType::Mount,
            ProfileConfig {
                name: "media".into(),
                app: Default::default(),
                rclone: json!({ "mountPoint": "/mnt/media" }),
            },
        );
        store.remotes.insert("drive".into(), meta);
        let mut home = MountedRemote::new("drive:", "/mnt/home");
        home.profile = "home".into();
        let mut media = MountedRemote::new("drive:", "/mnt/media");
        media.profile = "media".into();
        let plan = plan_tray(&remotes, &store, &[], &[home, media], &[], 4);
        let remote = plan
            .iter()
            .find(|item| item.label.contains("drive"))
            .expect("remote submenu");
        assert!(remote.label.starts_with('●'), "{}", remote.label);
        let browse = remote
            .children
            .iter()
            .find(|child| child.label == "Browse" && !child.children.is_empty())
            .expect("browse submenu");
        assert!(browse.children.iter().any(|child| {
            child.action
                == Some(TrayAction::BrowseRemote {
                    remote: "drive".into(),
                    profile: "home".into(),
                })
        }));
        assert!(browse.children.iter().any(|child| {
            child.action
                == Some(TrayAction::BrowseRemote {
                    remote: "drive".into(),
                    profile: "media".into(),
                })
        }));
        let flat = flatten_tray_menu(&plan);
        assert!(flat
            .iter()
            .any(|(_, action)| action.as_deref() == Some("browse|drive|home")));
        assert!(flat
            .iter()
            .any(|(_, action)| action.as_deref() == Some("browse|drive|media")));
    }
}
