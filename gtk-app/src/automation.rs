//! Cron + filesystem-watch automations built from saved profiles and quick runs.

use crate::operations::OperationType;
use crate::rclone::RcClient;
use crate::store::{AppStore, ProfileConfig, QuickRun};

pub use crate::store::{AutomationRuntime, AutomationStatus};
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
    pub destinations: Vec<String>,
    pub next_run: Option<DateTime<Utc>>,
    pub last_run: Option<DateTime<Utc>>,
    #[serde(default)]
    pub status: AutomationStatus,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub current_job_id: Option<String>,
    #[serde(default)]
    pub run_count: u64,
    #[serde(default)]
    pub success_count: u64,
    #[serde(default)]
    pub failure_count: u64,
    #[serde(default)]
    pub stopped_count: u64,
}

#[derive(Debug, Clone, Default)]
pub struct WatchPending {
    pub last_change: Option<std::time::Instant>,
    pub paths: std::collections::HashSet<String>,
}

pub fn local_watch_sources(store: &AppStore) -> Vec<String> {
    collect(store)
        .into_iter()
        .flat_map(|r| r.sources)
        .filter(|p| is_local_watch_path(p))
        .collect()
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
                out.push(with_runtime(
                    record_from_profile(
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
                    ),
                    store,
                ));
            }
        }
    }
    for qr in &store.quick_runs {
        if !qr.config.app.cron_enabled && !qr.config.app.watch_enabled {
            continue;
        }
        let id = format!("quick:{}", qr.id);
        out.push(with_runtime(
            record_from_quick(qr, store.automation_last_run.get(&id).cloned()),
            store,
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
    let (src, dst) = crate::store::quick_run_paths(&cfg.rclone, operation);
    let mut sources = split_paths(src);
    let destinations = split_paths(dst);
    if operation == OperationType::Bisync {
        for path in &destinations {
            if is_local_watch_path(path) && !sources.iter().any(|s| s == path) {
                sources.push(path.clone());
            }
        }
    }
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
        destinations,
        status: AutomationStatus::Enabled,
        last_error: None,
        current_job_id: None,
        run_count: 0,
        success_count: 0,
        failure_count: 0,
        stopped_count: 0,
    }
}

fn with_runtime(mut record: AutomationRecord, store: &AppStore) -> AutomationRecord {
    if let Some(runtime) = store.automation_runtime.get(&record.id) {
        record.status = runtime.status;
        record.last_error = runtime.last_error.clone();
        record.current_job_id = runtime.current_job_id.clone();
        record.run_count = runtime.run_count;
        record.success_count = runtime.success_count;
        record.failure_count = runtime.failure_count;
        record.stopped_count = runtime.stopped_count;
    }
    if store.is_automation_paused(&record.id) && record.status != AutomationStatus::Running {
        record.status = AutomationStatus::Disabled;
    }
    record
}

pub fn status_key(status: AutomationStatus) -> &'static str {
    match status {
        AutomationStatus::Enabled => "automation.status.enabled",
        AutomationStatus::Disabled => "automation.status.disabled",
        AutomationStatus::Running => "automation.status.running",
        AutomationStatus::Failed => "automation.status.failed",
        AutomationStatus::Stopping => "automation.status.stopping",
    }
}

pub fn is_quick_run(id: &str) -> bool {
    id.starts_with("quick:")
}

pub fn origin_label_key(id: &str) -> &'static str {
    if is_quick_run(id) {
        "generalOverview.jobs.originQuickRun"
    } else {
        "generalOverview.jobs.originDashboard"
    }
}

pub fn next_run_key(record: &AutomationRecord) -> Option<&'static str> {
    match record.status {
        AutomationStatus::Disabled => Some("automation.nextRun.disabled"),
        AutomationStatus::Stopping => Some("automation.nextRun.stopping"),
        _ if record.next_run.is_none() => Some("automation.nextRun.notScheduled"),
        _ => None,
    }
}

pub fn format_stamp(when: DateTime<Utc>) -> String {
    when.with_timezone(&chrono::Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

pub fn next_run_text(record: &AutomationRecord) -> String {
    record
        .next_run
        .map(format_stamp)
        .unwrap_or_else(|| "—".into())
}

pub fn last_run_text(record: &AutomationRecord) -> Option<String> {
    record.last_run.map(format_stamp)
}

pub fn stat_counts(record: &AutomationRecord) -> (u64, u64, u64, u64) {
    (
        record.success_count,
        record.failure_count,
        record.stopped_count,
        record.run_count,
    )
}

pub fn path_rows(record: &AutomationRecord) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    for path in &record.sources {
        if !path.is_empty() {
            rows.push(("dashboard.generalDetail.sourceLabel", path.clone()));
        }
    }
    for path in &record.destinations {
        if !path.is_empty() {
            rows.push(("dashboard.generalDetail.destinationLabel", path.clone()));
        }
    }
    rows
}

pub fn carousel_index(ids: &[String], selected: Option<&str>) -> usize {
    selected
        .and_then(|id| ids.iter().position(|item| item == id))
        .unwrap_or(0)
}

pub fn lifecycle_stats(record: &AutomationRecord) -> String {
    let mut bits = vec![
        format!("{} ok", record.success_count),
        format!("{} fail", record.failure_count),
        format!("{} runs", record.run_count),
    ];
    if record.stopped_count > 0 {
        bits.insert(2, format!("{} stop", record.stopped_count));
    }
    if let Some(job) = record.current_job_id.as_deref().filter(|id| !id.is_empty()) {
        bits.push(format!("job {job}"));
    }
    if let Some(error) = record.last_error.as_deref().filter(|e| !e.is_empty()) {
        bits.push(format!("error: {error}"));
    }
    bits.join(" · ")
}

pub fn can_run(runtime: &AutomationRuntime) -> bool {
    matches!(
        runtime.status,
        AutomationStatus::Enabled | AutomationStatus::Failed | AutomationStatus::Disabled
    ) && runtime.current_job_id.is_none()
}

pub fn mark_starting(runtime: &mut AutomationRuntime) -> Result<(), String> {
    if !can_run(runtime) {
        return Err(format!(
            "Automation cannot start from status {:?}",
            runtime.status
        ));
    }
    runtime.status = AutomationStatus::Running;
    runtime.current_job_id = None;
    runtime.run_count += 1;
    Ok(())
}

pub fn mark_running(runtime: &mut AutomationRuntime, job_id: String) {
    runtime.status = AutomationStatus::Running;
    runtime.current_job_id = Some(job_id);
}

pub fn mark_success(runtime: &mut AutomationRuntime) {
    runtime.last_error = None;
    runtime.current_job_id = None;
    runtime.success_count += 1;
    runtime.status = if runtime.status == AutomationStatus::Stopping {
        AutomationStatus::Disabled
    } else {
        AutomationStatus::Enabled
    };
}

pub fn mark_failure(runtime: &mut AutomationRuntime, error: String) {
    runtime.last_error = Some(error);
    runtime.current_job_id = None;
    runtime.failure_count += 1;
    runtime.status = if runtime.status == AutomationStatus::Stopping {
        AutomationStatus::Disabled
    } else {
        AutomationStatus::Failed
    };
}

pub fn mark_stopped(runtime: &mut AutomationRuntime) {
    runtime.current_job_id = None;
    runtime.stopped_count += 1;
    runtime.status = if runtime.status == AutomationStatus::Stopping {
        AutomationStatus::Disabled
    } else {
        AutomationStatus::Enabled
    };
}

pub fn automation_id_for_job(
    job: &crate::store::JobInfo,
    meta: Option<&crate::store::JobMeta>,
) -> Option<String> {
    if let Some(meta) = meta {
        if meta.origin == "automation" && !meta.quick_run_id.is_empty() {
            return Some(meta.quick_run_id.clone());
        }
    }
    if job.origin == "automation" && !job.profile.is_empty() && !job.remote.is_empty() {
        return Some(format!(
            "remote:{}:{}:{}",
            job.remote, job.operation, job.profile
        ));
    }
    None
}

pub fn apply_job_terminal(store: &mut AppStore, job: &crate::store::JobInfo) -> bool {
    let Some(id) = automation_id_for_job(job, store.job_meta.get(&job.id)) else {
        return false;
    };
    let runtime = store.automation_runtime.entry(id.clone()).or_default();
    match job.status.as_str() {
        "completed" => mark_success(runtime),
        "failed" => mark_failure(
            runtime,
            job.error
                .clone()
                .unwrap_or_else(|| "rclone job failed".into()),
        ),
        "stopped" => mark_stopped(runtime),
        _ => return false,
    }
    store.automation_last_run.insert(id, Utc::now());
    true
}

pub fn apply_job_transitions(
    store: &mut AppStore,
    previous: &[crate::store::JobInfo],
    current: &[crate::store::JobInfo],
) -> usize {
    let mut updated = 0;
    for job in current {
        let was = previous.iter().find(|item| item.id == job.id);
        let became_terminal = matches!(job.status.as_str(), "completed" | "failed" | "stopped")
            && was.map(|item| item.status.as_str()) != Some(job.status.as_str());
        if became_terminal && apply_job_terminal(store, job) {
            updated += 1;
        }
    }
    updated
}

fn split_paths(joined: Option<String>) -> Vec<String> {
    joined
        .map(|s| {
            s.split(", ")
                .filter(|p| !p.is_empty())
                .map(|p| p.to_string())
                .collect()
        })
        .unwrap_or_default()
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

pub fn note_watch_change(
    pending: &mut WatchPending,
    paths: impl IntoIterator<Item = String>,
) -> bool {
    let mut any = false;
    for path in paths {
        if path.is_empty() {
            continue;
        }
        pending.paths.insert(path);
        any = true;
    }
    if any {
        pending.last_change = Some(std::time::Instant::now());
    }
    any
}

pub fn watch_ready(pending: &WatchPending, delay_secs: u64, now: std::time::Instant) -> bool {
    let Some(last) = pending.last_change else {
        return false;
    };
    !pending.paths.is_empty()
        && now.duration_since(last) >= std::time::Duration::from_secs(delay_secs)
}

/// Join a rclone remote or local root with a relative subdirectory.
pub fn build_full_path(root: &str, rel: &str) -> String {
    let clean = rel.trim_start_matches('/');
    if clean.is_empty() {
        return root.to_string();
    }
    if root.ends_with(':') {
        let is_drive_letter =
            root.len() == 2 && root.starts_with(|c: char| c.is_ascii_alphabetic());
        if is_drive_letter {
            format!("{root}/{clean}")
        } else {
            format!("{root}{clean}")
        }
    } else {
        format!("{}/{clean}", root.trim_end_matches('/'))
    }
}

/// Map changed local paths to `(src, dst)` pairs (Tauri `compute_scoped_targets`).
pub fn compute_scoped_targets(
    src_paths: &[String],
    dst_paths: &[String],
    changed_paths: &std::collections::HashSet<String>,
) -> Vec<(String, String)> {
    if changed_paths.is_empty() || src_paths.is_empty() {
        return Vec::new();
    }
    let default_dst = dst_paths.first().cloned().unwrap_or_default();
    let mut scoped_pairs = std::collections::HashSet::new();
    for changed in changed_paths {
        let changed_path = Path::new(changed);
        let Some((idx, src_root)) = src_paths
            .iter()
            .enumerate()
            .find(|(_, src)| changed_path.starts_with(src.as_str()) || changed == *src)
        else {
            continue;
        };
        let src_root_path = Path::new(src_root);
        let rel_path = changed_path
            .strip_prefix(src_root_path)
            .unwrap_or(Path::new(""));
        let rel_dir = if changed_path.is_dir() || changed.ends_with('/') {
            rel_path.to_string_lossy().into_owned()
        } else {
            rel_path
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let rel_dir = rel_dir.replace('\\', "/").trim_matches('/').to_string();
        let target_dst_root = dst_paths.get(idx).unwrap_or(&default_dst);
        let (scoped_src, scoped_dst) = if rel_dir.is_empty() {
            (src_root.clone(), target_dst_root.clone())
        } else {
            (
                build_full_path(src_root, &rel_dir),
                build_full_path(target_dst_root, &rel_dir),
            )
        };
        scoped_pairs.insert((scoped_src, scoped_dst));
    }
    let mut result: Vec<(String, String)> = scoped_pairs.into_iter().collect();
    result.sort();
    result
}

pub fn fire(
    client: &RcClient,
    store: &mut AppStore,
    record: &AutomationRecord,
    now: DateTime<Utc>,
    scoped: Option<&[(String, String)]>,
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
    {
        if store.is_automation_paused(&record.id) {
            return Err("automation paused".into());
        }
        let runtime = store
            .automation_runtime
            .entry(record.id.clone())
            .or_default();
        mark_starting(runtime)?;
    }
    let result = crate::jobs::start_profile_ex(
        client,
        &record.remote,
        record.operation,
        &profile,
        meta.as_ref(),
        "automation",
        scoped,
    );
    match result {
        Ok(result) => {
            let ids = crate::jobs::parse_started_ids(&result);
            let runtime = store
                .automation_runtime
                .entry(record.id.clone())
                .or_default();
            if let Some(id) = ids.first() {
                mark_running(runtime, id.to_string());
            }
            crate::jobs::remember_started(
                &mut store.job_meta,
                &result,
                crate::jobs::job_meta_for(&record.remote, &profile, "automation", "", &record.id),
            );
            store.automation_last_run.insert(record.id.clone(), now);
            Ok(result)
        }
        Err(error) => {
            let runtime = store
                .automation_runtime
                .entry(record.id.clone())
                .or_default();
            mark_failure(runtime, error.clone());
            store.automation_last_run.insert(record.id.clone(), now);
            Err(error)
        }
    }
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
        assert!(local_watch_sources(&store).iter().any(|p| p == "/tmp"));
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

    #[test]
    fn scoped_targets_map_child_file_to_parent_dir() {
        let src = vec!["/tmp/docs".into()];
        let dst = vec!["testdrive:out".into()];
        let mut changed = std::collections::HashSet::new();
        changed.insert("/tmp/docs/Photos/IMG.jpg".into());
        assert_eq!(
            compute_scoped_targets(&src, &dst, &changed),
            vec![("/tmp/docs/Photos".into(), "testdrive:out/Photos".into())]
        );
    }

    #[test]
    fn scoped_targets_ignore_unrelated_paths() {
        let src = vec!["/tmp/docs".into()];
        let dst = vec!["testdrive:out".into()];
        let mut changed = std::collections::HashSet::new();
        changed.insert("/tmp/other/file.txt".into());
        assert!(compute_scoped_targets(&src, &dst, &changed).is_empty());
    }

    #[test]
    fn watch_ready_respects_debounce_delay() {
        let mut pending = WatchPending::default();
        assert!(!watch_ready(&pending, 0, std::time::Instant::now()));
        note_watch_change(&mut pending, ["/tmp/docs/a.txt".into()]);
        assert!(watch_ready(&pending, 0, std::time::Instant::now()));
        pending.last_change = Some(std::time::Instant::now());
        assert!(!watch_ready(&pending, 5, std::time::Instant::now()));
        pending.last_change = Some(std::time::Instant::now() - std::time::Duration::from_secs(6));
        assert!(watch_ready(&pending, 5, std::time::Instant::now()));
    }

    #[test]
    fn build_full_path_joins_remote_and_local() {
        assert_eq!(build_full_path("testdrive:", "Photos"), "testdrive:Photos");
        assert_eq!(
            build_full_path("testdrive:out", "Photos"),
            "testdrive:out/Photos"
        );
        assert_eq!(build_full_path("/tmp/docs", "Photos"), "/tmp/docs/Photos");
        assert_eq!(build_full_path("C:", "Users"), "C:/Users");
    }

    fn runtime_ready() -> AutomationRuntime {
        AutomationRuntime {
            status: AutomationStatus::Enabled,
            ..AutomationRuntime::default()
        }
    }

    #[test]
    fn marks_success_failure_and_stopped() {
        let mut runtime = runtime_ready();
        mark_starting(&mut runtime).unwrap();
        mark_running(&mut runtime, "42".into());
        assert_eq!(runtime.status, AutomationStatus::Running);
        assert_eq!(runtime.current_job_id.as_deref(), Some("42"));
        assert_eq!(runtime.run_count, 1);
        mark_success(&mut runtime);
        assert_eq!(runtime.status, AutomationStatus::Enabled);
        assert_eq!(runtime.success_count, 1);
        assert!(runtime.current_job_id.is_none());
        assert!(runtime.last_error.is_none());

        mark_starting(&mut runtime).unwrap();
        mark_failure(&mut runtime, "disk full".into());
        assert_eq!(runtime.status, AutomationStatus::Failed);
        assert_eq!(runtime.failure_count, 1);
        assert_eq!(runtime.last_error.as_deref(), Some("disk full"));

        runtime.status = AutomationStatus::Running;
        mark_stopped(&mut runtime);
        assert_eq!(runtime.status, AutomationStatus::Enabled);
        assert_eq!(runtime.stopped_count, 1);
    }

    #[test]
    fn refuses_second_start_while_running() {
        let mut runtime = runtime_ready();
        mark_starting(&mut runtime).unwrap();
        assert!(mark_starting(&mut runtime).is_err());
    }

    #[test]
    fn stopping_disables_on_terminal() {
        let mut runtime = runtime_ready();
        runtime.status = AutomationStatus::Stopping;
        mark_success(&mut runtime);
        assert_eq!(runtime.status, AutomationStatus::Disabled);

        runtime.status = AutomationStatus::Stopping;
        mark_failure(&mut runtime, "no".into());
        assert_eq!(runtime.status, AutomationStatus::Disabled);
    }

    #[test]
    fn collect_merges_runtime_and_pause() {
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
        let id = "remote:drive:sync:nightly";
        store.automation_runtime.insert(
            id.into(),
            AutomationRuntime {
                status: AutomationStatus::Failed,
                last_error: Some("quota".into()),
                success_count: 2,
                failure_count: 1,
                run_count: 3,
                ..AutomationRuntime::default()
            },
        );
        let items = collect(&store);
        assert_eq!(items[0].status, AutomationStatus::Failed);
        assert_eq!(items[0].failure_count, 1);
        assert_eq!(items[0].last_error.as_deref(), Some("quota"));
        store.automation_paused.push(id.into());
        let paused = collect(&store);
        assert_eq!(paused[0].status, AutomationStatus::Disabled);
        assert_eq!(paused[0].failure_count, 1);
        assert_eq!(paused[0].last_error.as_deref(), Some("quota"));
    }

    #[test]
    fn job_completion_updates_runtime() {
        let mut store = AppStore::default();
        store.job_meta.insert(
            9,
            crate::store::JobMeta {
                origin: "automation".into(),
                quick_run_id: "quick:abc".into(),
                ..crate::store::JobMeta::default()
            },
        );
        let running = crate::store::JobInfo {
            id: 9,
            operation: "copy".into(),
            remote: "drive".into(),
            profile: "nightly".into(),
            status: "running".into(),
            origin: "automation".into(),
            start_time: Utc::now(),
            error: None,
            dry_run: false,
            src: String::new(),
            dst: String::new(),
            group: String::new(),
            stats: json!({}),
            transferring: json!([]),
            duration: 0.0,
            progress: 0.0,
            output: json!({}),
            completed: json!([]),
            parent_job_id: None,
        };
        let mut failed = running.clone();
        failed.status = "failed".into();
        failed.error = Some("network".into());
        assert_eq!(apply_job_transitions(&mut store, &[running], &[failed]), 1);
        let runtime = &store.automation_runtime["quick:abc"];
        assert_eq!(runtime.status, AutomationStatus::Failed);
        assert_eq!(runtime.failure_count, 1);
        assert_eq!(runtime.last_error.as_deref(), Some("network"));
    }

    fn sample_record() -> AutomationRecord {
        AutomationRecord {
            id: "remote:drive:sync:nightly".into(),
            name: "drive / sync / nightly".into(),
            remote: "drive".into(),
            profile: "nightly".into(),
            operation: OperationType::Sync,
            cron: "0 2 * * *".into(),
            cron_enabled: true,
            watch_enabled: false,
            watch_delay: 5,
            watch_changed_only: false,
            sources: vec!["drive:src".into(), "/tmp/docs".into()],
            destinations: vec!["/backup".into()],
            next_run: Some(Utc.with_ymd_and_hms(2026, 8, 27, 2, 0, 0).unwrap()),
            last_run: Some(Utc.with_ymd_and_hms(2026, 8, 26, 2, 0, 0).unwrap()),
            status: AutomationStatus::Failed,
            last_error: Some("quota".into()),
            current_job_id: None,
            run_count: 4,
            success_count: 3,
            failure_count: 1,
            stopped_count: 0,
        }
    }

    #[test]
    fn card_helpers_split_stats_paths_and_schedule() {
        let record = sample_record();
        assert!(!is_quick_run(&record.id));
        assert_eq!(
            origin_label_key("quick:abc"),
            "generalOverview.jobs.originQuickRun"
        );
        assert_eq!(stat_counts(&record), (3, 1, 0, 4));
        assert_eq!(
            path_rows(&record),
            vec![
                ("dashboard.generalDetail.sourceLabel", "drive:src".into()),
                ("dashboard.generalDetail.sourceLabel", "/tmp/docs".into()),
                ("dashboard.generalDetail.destinationLabel", "/backup".into()),
            ]
        );
        assert!(next_run_key(&record).is_none());
        assert!(next_run_text(&record).contains("2026-08-27"));
        assert!(last_run_text(&record).unwrap().contains("2026-08-26"));
        let mut disabled = record.clone();
        disabled.status = AutomationStatus::Disabled;
        assert_eq!(next_run_key(&disabled), Some("automation.nextRun.disabled"));
        let mut idle = record.clone();
        idle.next_run = None;
        idle.status = AutomationStatus::Enabled;
        assert_eq!(next_run_key(&idle), Some("automation.nextRun.notScheduled"));
        assert_eq!(
            carousel_index(&["a".into(), "b".into(), "c".into()], Some("b")),
            1
        );
        assert_eq!(carousel_index(&["a".into(), "b".into()], None), 0);
    }
}
