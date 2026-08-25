//! Plan StatusNotifier tray menus from runtime state (testable, no GTK).

use crate::jobs::{find_active_quick_run, profile_is_active};
use crate::operations::OperationType;
use crate::rclone::{MountedRemote, ServeItem};
use crate::store::{AppStore, JobInfo, RemoteInfo};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    ShowWindow,
    OpenFiles,
    Quit,
    UnmountAll,
    StopJobs,
    StopServes,
    MountRemote {
        remote: String,
        profile: String,
    },
    UnmountRemote {
        remote: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayMenuItem {
    pub label: String,
    pub enabled: bool,
    pub action: Option<TrayAction>,
    pub children: Vec<TrayMenuItem>,
}

impl TrayMenuItem {
    pub fn action(label: impl Into<String>, action: TrayAction) -> Self {
        Self {
            label: label.into(),
            enabled: true,
            action: Some(action),
            children: Vec::new(),
        }
    }

    pub fn disabled(label: impl Into<String>, action: TrayAction) -> Self {
        Self {
            label: label.into(),
            enabled: false,
            action: Some(action),
            children: Vec::new(),
        }
    }

    pub fn submenu(label: impl Into<String>, children: Vec<Self>) -> Self {
        Self {
            label: label.into(),
            enabled: true,
            action: None,
            children,
        }
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
        TrayMenuItem::action("Quit", TrayAction::Quit),
        TrayMenuItem::action("Unmount All", TrayAction::UnmountAll),
        TrayMenuItem::action("Stop All Jobs", TrayAction::StopJobs),
        TrayMenuItem::action("Stop All Serves", TrayAction::StopServes),
    ];
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
    for remote in visible {
        items.push(remote_submenu(remote, store, jobs, mounts, serves));
    }
    let quick: Vec<_> = store
        .quick_runs
        .iter()
        .filter(|q| q.show_on_tray)
        .cloned()
        .collect();
    if !quick.is_empty() {
        let mut children = Vec::new();
        for qr in quick {
            let active = find_active_quick_run(jobs, &qr).is_some();
            if active {
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
        items.push(TrayMenuItem::submenu("Quick Runs", children));
    }
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
    let mounted = remote.mounted;
    let mut children = vec![
        if mounted {
            TrayMenuItem::disabled(
                format!("Mount {name}"),
                TrayAction::MountRemote {
                    remote: name.clone(),
                    profile: "default".into(),
                },
            )
        } else {
            TrayMenuItem::action(
                format!("Mount {name}"),
                TrayAction::MountRemote {
                    remote: name.clone(),
                    profile: "default".into(),
                },
            )
        },
        if mounted {
            TrayMenuItem::action(
                format!("Unmount {name}"),
                TrayAction::UnmountRemote {
                    remote: name.clone(),
                },
            )
        } else {
            TrayMenuItem::disabled(
                format!("Unmount {name}"),
                TrayAction::UnmountRemote {
                    remote: name.clone(),
                },
            )
        },
        TrayMenuItem::action("Browse", TrayAction::BrowseRemote(name.clone())),
        TrayMenuItem::action("Browse in app", TrayAction::BrowseInApp(name.clone())),
    ];
    if let Some(meta) = store.remotes.get(&name) {
        for op in meta.visible_operations() {
            if matches!(op, OperationType::Mount) {
                continue;
            }
            let names = meta.profile_names(op);
            let profiles = if names.is_empty() {
                vec!["default".into()]
            } else {
                names
            };
            for pname in profiles {
                let active = profile_is_active(&name, op, &pname, jobs, mounts, serves);
                let label = if active {
                    format!("Stop {} · {pname}", op.api_label())
                } else {
                    format!("Start {} · {pname}", op.api_label())
                };
                let action = if active {
                    TrayAction::StopProfile {
                        remote: name.clone(),
                        op: op.as_str().into(),
                        profile: pname,
                    }
                } else {
                    TrayAction::StartProfile {
                        remote: name.clone(),
                        op: op.as_str().into(),
                        profile: pname,
                    }
                };
                children.push(TrayMenuItem::action(label, action));
            }
        }
    }
    TrayMenuItem::submenu(if mounted { format!("● {name}") } else { name }, children)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ProfileConfig, QuickRun, RemoteMeta};
    use serde_json::json;

    #[test]
    fn plans_profile_and_quick_run_actions() {
        let remotes = vec![RemoteInfo {
            name: "drive".into(),
            r#type: "drive".into(),
            mounted: true,
            serving: false,
            job_active: true,
            hidden: false,
            disk_label: String::new(),
        }];
        let mut store = AppStore::default();
        let mut meta = RemoteMeta {
            show_on_tray: true,
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
        }];
        let mounts = vec![MountedRemote {
            fs: "drive:".into(),
            mount_point: "/mnt/drive".into(),
        }];
        let plan = plan_tray(&remotes, &store, &jobs, &mounts, &[], 8);
        assert!(plan.iter().any(|i| i.action == Some(TrayAction::Quit)));
        let remote = plan
            .iter()
            .find(|i| i.label.contains("drive"))
            .expect("remote submenu");
        assert!(remote.children.iter().any(|c| matches!(
            c.action,
            Some(TrayAction::StartProfile { ref op, ref profile, .. })
                if op == "sync" && profile == "nightly"
        )));
        assert!(remote
            .children
            .iter()
            .any(|c| matches!(c.action, Some(TrayAction::UnmountRemote { .. }))));
        let quick = plan.iter().find(|i| i.label == "Quick Runs").unwrap();
        assert_eq!(
            quick.children[0].action,
            Some(TrayAction::StopQuickRun(store.quick_runs[0].id.clone()))
        );
    }
}
