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
    BrowseRemote(String),
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
        children.push(TrayMenuItem::action(
            "Browse",
            TrayAction::BrowseRemote(name.clone()),
        ));
    } else {
        children.push(TrayMenuItem::action(
            "Browse in app",
            TrayAction::BrowseInApp(name.clone()),
        ));
    }

    let label = if mounted { format!("● {name}") } else { name };
    TrayMenuItem::submenu(label, children)
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
}
