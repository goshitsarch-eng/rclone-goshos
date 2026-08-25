//! Cron + filesystem-watch automations built from saved profiles and quick runs.

use crate::jobs::start_profile;
use crate::operations::OperationType;
use crate::rclone::RcClient;
use crate::store::{AppStore, ProfileConfig, QuickRun};
use chrono::{DateTime, Utc};
use croner::Cron;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationRecord {
    pub id: String,
    pub name: String,
    pub remote: String,
    pub profile: String,
    pub operation: OperationType,
    pub cron: String,
    pub cron_enabled: bool,
    pub watch_enabled: bool,
    pub watch_delay: u64,
    pub watch_changed_only: bool,
    pub sources: Vec<String>,
    pub next_run: Option<DateTime<Utc>>,
    pub last_run: Option<DateTime<Utc>>,
}

pub fn collect(store: &AppStore) -> Vec<AutomationRecord> {
    let mut out = Vec::new();
    for (remote, meta) in &store.remotes {
        for (op_key, profiles) in &meta.profiles {
            let Some(op) = OperationType::parse(op_key) else {
                continue;
            };
            for (pname, profile) in profiles {
                if !profile.app.cron_enabled && !profile.app.watch_enabled {
                    continue;
                }
                let id = format!("remote:{remote}:{op_key}:{pname}");
                out.push(record_from_profile(
                    id,
                    format!("{remote} / {op_key} / {pname}"),
                    remote.clone(),
                    pname.clone(),
                    op,
                    profile,
                    store
                        .automation_last_run
                        .get(&format!("remote:{remote}:{op_key}:{pname}"))
                        .cloned(),
                ));
            }
        }
    }
    for qr in &store.quick_runs {
        if !qr.config.app.cron_enabled && !qr.config.app.watch_enabled {
            continue;
        }
        let id = format!("quick:{}", qr.id);
        out.push(record_from_quick(
            qr,
            store.automation_last_run.get(&id).cloned(),
        ));
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn record_from_profile(
    id: String,
    name: String,
    remote: String,
    profile: String,
    operation: OperationType,
    cfg: &ProfileConfig,
    last_run: Option<DateTime<Utc>>,
) -> AutomationRecord {
    let (src, _) = crate::store::quick_run_paths(&cfg.rclone, operation);
    let sources = src
        .map(|s| {
            s.split(", ")
                .filter(|p| !p.is_empty())
                .map(|p| p.to_string())
                .collect()
        })
        .unwrap_or_default();
    AutomationRecord {
        next_run: if cfg.app.cron_enabled {
            next_cron_run(&cfg.app.cron_expression, Utc::now())
        } else {
            None
        },
        last_run,
        id,
        name,
        remote,
        profile,
        operation,
        cron: cfg.app.cron_expression.clone(),
        cron_enabled: cfg.app.cron_enabled,
        watch_enabled: cfg.app.watch_enabled,
        watch_delay: cfg.app.watch_delay,
        watch_changed_only: cfg.app.watch_changed_only,
        sources,
    }
}

fn record_from_quick(qr: &QuickRun, last_run: Option<DateTime<Utc>>) -> AutomationRecord {
    record_from_profile(
        format!("quick:{}", qr.id),
        qr.name.clone(),
        qr.remote_name.clone(),
        qr.config.name.clone(),
        qr.operation_type,
        &qr.config,
        last_run,
    )
}

pub fn next_cron_run(expr: &str, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let cron = Cron::new(expr).parse().ok()?;
    let local = from.with_timezone(&chrono::Local);
    cron.find_next_occurrence(&local, false)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

pub fn cron_is_due(expr: &str, last_run: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    if expr.trim().is_empty() {
        return false;
    }
    let Some(next_after_last) = next_cron_run(
        expr,
        last_run.unwrap_or_else(|| now - chrono::Duration::minutes(2)),
    ) else {
        return false;
    };
    next_after_last <= now
}

pub fn is_local_watch_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    path.starts_with('/') || Path::new(path).exists()
}

pub fn path_mtime(path: &str) -> Option<u64> {
    let meta = std::fs::metadata(path).ok()?;
    let mut best = meta.modified().ok()?;
    if meta.is_dir() {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                if let Ok(child) = entry.metadata() {
                    if let Ok(modified) = child.modified() {
                        if modified > best {
                            best = modified;
                        }
                    }
                }
            }
        }
    }
    best.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

pub fn watch_triggered(
    sources: &[String],
    last_mtime: &mut HashMap<String, u64>,
    changed_only: bool,
) -> bool {
    let mut triggered = false;
    for path in sources.iter().filter(|p| is_local_watch_path(p)) {
        let Some(mtime) = path_mtime(path) else {
            continue;
        };
        match last_mtime.get(path) {
            None => {
                last_mtime.insert(path.clone(), mtime);
                if !changed_only {
                    triggered = true;
                }
            }
            Some(prev) if *prev < mtime => {
                last_mtime.insert(path.clone(), mtime);
                triggered = true;
            }
            _ => {}
        }
    }
    triggered
}

pub fn fire(
    client: &RcClient,
    store: &mut AppStore,
    record: &AutomationRecord,
    now: DateTime<Utc>,
) -> Result<String, String> {
    let profile = if let Some(qr_id) = record.id.strip_prefix("quick:") {
        store
            .quick_runs
            .iter()
            .find(|q| q.id == qr_id)
            .map(|q| q.config.clone())
            .ok_or_else(|| "quick run missing".to_string())?
    } else {
        store
            .remotes
            .get(&record.remote)
            .and_then(|m| m.profiles.get(record.operation.as_str()))
            .and_then(|m| m.get(&record.profile))
            .cloned()
            .ok_or_else(|| "profile missing".to_string())?
    };
    let meta = store.remotes.get(&record.remote).cloned();
    let result = start_profile(
        client,
        &record.remote,
        record.operation,
        &profile,
        meta.as_ref(),
    )?;
    store.automation_last_run.insert(record.id.clone(), now);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{AppConfig, RemoteMeta};
    use chrono::TimeZone;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn collects_cron_profiles_and_quick_runs() {
        let mut store = AppStore::default();
        let mut meta = RemoteMeta::default();
        let mut profiles = HashMap::new();
        profiles.insert(
            "nightly".into(),
            ProfileConfig {
                name: "nightly".into(),
                app: AppConfig {
                    cron_enabled: true,
                    cron_expression: "0 2 * * *".into(),
                    ..AppConfig::default()
                },
                rclone: json!({ "srcFs": "drive:src", "dstFs": "/tmp" }),
            },
        );
        meta.profiles.insert("sync".into(), profiles);
        store.remotes.insert("drive".into(), meta);
        let mut qr = QuickRun::new("watch docs".into(), OperationType::Copy, "drive".into());
        qr.config.app.watch_enabled = true;
        qr.config.rclone = json!({ "srcFs": "/tmp", "dstFs": "drive:out" });
        store.quick_runs.push(qr);
        let items = collect(&store);
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|a| a.cron_enabled));
        assert!(items.iter().any(|a| a.watch_enabled));
    }

    #[test]
    fn next_run_is_in_the_future() {
        let now = Utc.with_ymd_and_hms(2026, 8, 25, 10, 0, 0).unwrap();
        let next = next_cron_run("0 11 * * *", now).unwrap();
        assert!(next > now);
    }

    #[test]
    fn due_when_slot_passed() {
        let now = Utc.with_ymd_and_hms(2026, 8, 25, 11, 0, 5).unwrap();
        let last = Utc.with_ymd_and_hms(2026, 8, 25, 10, 0, 0).unwrap();
        assert!(cron_is_due("0 11 * * *", Some(last), now));
        assert!(!cron_is_due(
            "0 11 * * *",
            Some(now),
            now + chrono::Duration::seconds(10)
        ));
    }

    #[test]
    fn local_watch_paths() {
        assert!(is_local_watch_path("/tmp"));
        assert!(!is_local_watch_path("drive:Photos"));
        assert!(!is_local_watch_path(""));
    }

    #[test]
    fn watch_detects_mtime_change() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "one").unwrap();
        let path = file.to_string_lossy().to_string();
        let mut seen = HashMap::new();
        seen.insert(path.clone(), 1);
        assert!(watch_triggered(&[path], &mut seen, true));
    }

    #[test]
    fn watch_detects_child_file_in_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        let mut seen = HashMap::new();
        seen.insert(path.clone(), 1);
        std::fs::write(dir.path().join("child.txt"), "changed").unwrap();
        assert!(watch_triggered(&[path], &mut seen, true));
    }
}
