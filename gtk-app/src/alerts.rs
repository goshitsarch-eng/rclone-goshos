//! Runtime diffs that turn mount/serve/job/automation/update changes into alert events.

use crate::rclone::{MountedRemote, ServeItem};
use crate::store::{AlertEvent, AlertEventKind, AlertSeverity, JobInfo};

pub fn job_event(
    job: &JobInfo,
    severity: AlertSeverity,
    title: String,
    body: String,
) -> AlertEvent {
    let mut event = AlertEvent::new(AlertEventKind::Job, severity, title, body);
    event.remote = job.remote.clone();
    event.origin = job.origin.clone();
    event.profile = job.profile.clone();
    event
}

pub fn job_events(previous: &[JobInfo], current: &[JobInfo]) -> Vec<AlertEvent> {
    let mut out = Vec::new();
    for job in current {
        let was = previous.iter().find(|j| j.id == job.id);
        if job.status == "failed" && was.map(|j| j.status.as_str()) != Some("failed") {
            out.push(job_event(
                job,
                AlertSeverity::High,
                format!("Job #{} failed", job.id),
                job.error
                    .clone()
                    .unwrap_or_else(|| "rclone job failed".into()),
            ));
        } else if job.status == "completed" && was.map(|j| j.status.as_str()) == Some("running") {
            out.push(job_event(
                job,
                AlertSeverity::Info,
                format!("Job #{} completed", job.id),
                format!("{} finished", job.operation),
            ));
        }
    }
    out
}

pub fn mount_events(previous: &[MountedRemote], current: &[MountedRemote]) -> Vec<AlertEvent> {
    let mut out = Vec::new();
    for mount in current {
        if !previous
            .iter()
            .any(|p| p.fs == mount.fs && p.mount_point == mount.mount_point)
        {
            let mut event = AlertEvent::new(
                AlertEventKind::Mount,
                AlertSeverity::Info,
                format!("Mounted {}", mount.fs),
                format!("{} → {}", mount.fs, mount.mount_point),
            );
            event.remote = infer_remote(&mount.fs);
            event.origin = "mount".into();
            out.push(event);
        }
    }
    for mount in previous {
        if !current
            .iter()
            .any(|c| c.fs == mount.fs && c.mount_point == mount.mount_point)
        {
            let mut event = AlertEvent::new(
                AlertEventKind::Mount,
                AlertSeverity::Warning,
                format!("Unmounted {}", mount.fs),
                format!("{} left {}", mount.fs, mount.mount_point),
            );
            event.remote = infer_remote(&mount.fs);
            event.origin = "mount".into();
            out.push(event);
        }
    }
    out
}

pub fn serve_events(previous: &[ServeItem], current: &[ServeItem]) -> Vec<AlertEvent> {
    let mut out = Vec::new();
    for serve in current {
        if !previous.iter().any(|p| p.id == serve.id) {
            let mut event = AlertEvent::new(
                AlertEventKind::Serve,
                AlertSeverity::Info,
                format!("Serving {} on {}", serve.serve_type, serve.addr),
                format!("{} · {}", serve.fs, serve.addr),
            );
            event.remote = infer_remote(&serve.fs);
            event.origin = "serve".into();
            out.push(event);
        }
    }
    for serve in previous {
        if !current.iter().any(|c| c.id == serve.id) {
            let mut event = AlertEvent::new(
                AlertEventKind::Serve,
                AlertSeverity::Warning,
                format!("Stopped {} serve", serve.serve_type),
                format!("{} · {}", serve.fs, serve.addr),
            );
            event.remote = infer_remote(&serve.fs);
            event.origin = "serve".into();
            out.push(event);
        }
    }
    out
}

pub fn automation_event(name: &str, remote: &str, ok: bool, detail: &str) -> AlertEvent {
    let mut event = AlertEvent::new(
        AlertEventKind::Automation,
        if ok {
            AlertSeverity::Info
        } else {
            AlertSeverity::High
        },
        if ok {
            format!("Automation {name} started")
        } else {
            format!("Automation {name} failed")
        },
        detail.to_string(),
    );
    event.remote = remote.to_string();
    event.origin = "automation".into();
    event
}

pub fn update_event(kind: &str, version: &str, url: &str) -> AlertEvent {
    let mut event = AlertEvent::new(
        AlertEventKind::Update,
        AlertSeverity::Info,
        format!("{kind} update available"),
        format!("{version} · {url}"),
    );
    event.origin = "updater".into();
    event
}

pub fn engine_event(online: bool, detail: &str) -> AlertEvent {
    AlertEvent::new(
        AlertEventKind::Engine,
        if online {
            AlertSeverity::Info
        } else {
            AlertSeverity::High
        },
        if online {
            "rclone engine online".into()
        } else {
            "rclone engine offline".into()
        },
        detail.to_string(),
    )
}

fn infer_remote(fs: &str) -> String {
    crate::rclone::split_remote_path(fs).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn job(id: u64, status: &str) -> JobInfo {
        JobInfo {
            id,
            operation: "sync".into(),
            remote: "drive".into(),
            profile: "default".into(),
            status: status.into(),
            origin: "dashboard".into(),
            start_time: Utc::now(),
            error: if status == "failed" {
                Some("boom".into())
            } else {
                None
            },
            dry_run: false,
            src: "drive:".into(),
            dst: "/tmp".into(),
            group: format!("job/{id}"),
            stats: serde_json::json!({}),
            transferring: serde_json::json!([]),
            duration: 1.0,
            progress: 1.0,
            output: serde_json::json!({}),
            completed: serde_json::json!([]),
        }
    }

    #[test]
    fn job_events_emit_fail_and_complete() {
        let prev = vec![job(1, "running"), job(2, "running")];
        let curr = vec![job(1, "failed"), job(2, "completed")];
        let events = job_events(&prev, &curr);
        assert_eq!(events.len(), 2);
        assert!(events.iter().any(|e| e.severity == AlertSeverity::High));
        assert!(events.iter().any(|e| e.title.contains("completed")));
    }

    #[test]
    fn mount_and_serve_start_and_stop() {
        let started = mount_events(
            &[],
            &[MountedRemote {
                fs: "drive:".into(),
                mount_point: "/mnt/drive".into(),
            }],
        );
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].kind, AlertEventKind::Mount);
        assert_eq!(started[0].remote, "drive");
        let stopped = serve_events(
            &[ServeItem {
                id: "s1".into(),
                addr: "127.0.0.1:8080".into(),
                fs: "box:".into(),
                serve_type: "http".into(),
            }],
            &[],
        );
        assert_eq!(stopped[0].kind, AlertEventKind::Serve);
        assert_eq!(stopped[0].severity, AlertSeverity::Warning);
        assert_eq!(stopped[0].remote, "box");
    }

    #[test]
    fn automation_and_update_kinds() {
        let ok = automation_event("nightly", "drive", true, "#12");
        assert_eq!(ok.kind, AlertEventKind::Automation);
        assert_eq!(ok.severity, AlertSeverity::Info);
        let fail = automation_event("nightly", "drive", false, "offline");
        assert_eq!(fail.severity, AlertSeverity::High);
        let update = update_event("app", "v2.0.0", "https://example");
        assert_eq!(update.kind, AlertEventKind::Update);
        let engine = engine_event(false, "rcd exited");
        assert_eq!(engine.kind, AlertEventKind::Engine);
    }
}
