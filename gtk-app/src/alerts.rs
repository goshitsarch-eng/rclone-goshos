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

pub type FormatFn<'a> = &'a dyn Fn(&str, &[(&str, &str)]) -> String;

pub fn job_events(previous: &[JobInfo], current: &[JobInfo], tf: FormatFn<'_>) -> Vec<AlertEvent> {
    let mut out = Vec::new();
    for job in current {
        let was = previous.iter().find(|j| j.id == job.id);
        let params = [
            ("type", job.operation.as_str()),
            ("remote", job.remote.as_str()),
            ("profile", job.profile.as_str()),
            ("backend", job.origin.as_str()),
            ("error", job.error.as_deref().unwrap_or("rclone job failed")),
        ];
        if job.status == "failed" && was.map(|j| j.status.as_str()) != Some("failed") {
            out.push(job_event(
                job,
                AlertSeverity::High,
                tf("notification.title.jobFailed", &params),
                tf("notification.body.jobFailed", &params),
            ));
        } else if job.status == "completed" && was.map(|j| j.status.as_str()) == Some("running") {
            out.push(job_event(
                job,
                AlertSeverity::Info,
                tf("notification.title.jobCompleted", &params),
                tf("notification.body.jobCompleted", &params),
            ));
        }
    }
    out
}

pub fn mount_events(
    previous: &[MountedRemote],
    current: &[MountedRemote],
    tf: FormatFn<'_>,
) -> Vec<AlertEvent> {
    let mut out = Vec::new();
    for mount in current {
        if !previous
            .iter()
            .any(|p| p.fs == mount.fs && p.mount_point == mount.mount_point)
        {
            let remote = infer_remote(&mount.fs);
            let params = [
                ("remote", remote.as_str()),
                ("profile", "default"),
                ("backend", "local"),
                ("mountPoint", mount.mount_point.as_str()),
            ];
            let mut event = AlertEvent::new(
                AlertEventKind::Mount,
                AlertSeverity::Info,
                tf("notification.title.mountSucceeded", &params),
                tf("notification.body.mountSucceeded", &params),
            );
            event.remote = remote;
            event.origin = "mount".into();
            out.push(event);
        }
    }
    for mount in previous {
        if !current
            .iter()
            .any(|c| c.fs == mount.fs && c.mount_point == mount.mount_point)
        {
            let remote = infer_remote(&mount.fs);
            let params = [
                ("remote", remote.as_str()),
                ("profile", "default"),
                ("backend", "local"),
                ("mountPoint", mount.mount_point.as_str()),
            ];
            let mut event = AlertEvent::new(
                AlertEventKind::Mount,
                AlertSeverity::Warning,
                tf("notification.title.unmountSucceeded", &params),
                tf("notification.body.unmountSucceeded", &params),
            );
            event.remote = remote;
            event.origin = "mount".into();
            out.push(event);
        }
    }
    out
}

pub fn serve_events(
    previous: &[ServeItem],
    current: &[ServeItem],
    tf: FormatFn<'_>,
) -> Vec<AlertEvent> {
    let mut out = Vec::new();
    for serve in current {
        if !previous.iter().any(|p| p.id == serve.id) {
            let remote = infer_remote(&serve.fs);
            let params = [
                ("remote", remote.as_str()),
                ("profile", serve.profile.as_str()),
                ("backend", "local"),
                ("type", serve.serve_type.as_str()),
            ];
            let mut event = AlertEvent::new(
                AlertEventKind::Serve,
                AlertSeverity::Info,
                tf("notification.title.serveStarted", &params),
                tf("notification.body.serveStarted", &params),
            );
            event.remote = remote;
            event.origin = "serve".into();
            out.push(event);
        }
    }
    for serve in previous {
        if !current.iter().any(|c| c.id == serve.id) {
            let remote = infer_remote(&serve.fs);
            let params = [
                ("remote", remote.as_str()),
                ("profile", serve.profile.as_str()),
                ("backend", "local"),
                ("type", serve.serve_type.as_str()),
            ];
            let mut event = AlertEvent::new(
                AlertEventKind::Serve,
                AlertSeverity::Warning,
                tf("notification.title.serveStopped", &params),
                tf("notification.body.serveStopped", &params),
            );
            event.remote = remote;
            event.origin = "serve".into();
            out.push(event);
        }
    }
    out
}

pub fn automation_event(
    name: &str,
    remote: &str,
    ok: bool,
    detail: &str,
    tf: FormatFn<'_>,
) -> AlertEvent {
    let params = [
        ("type", "cron"),
        ("automation", name),
        ("backend", "local"),
        ("error", detail),
    ];
    let mut event = AlertEvent::new(
        AlertEventKind::Automation,
        if ok {
            AlertSeverity::Info
        } else {
            AlertSeverity::High
        },
        tf(
            if ok {
                "notification.title.automationStarted"
            } else {
                "notification.title.automationFailed"
            },
            &params,
        ),
        if ok {
            tf("notification.body.automationStarted", &params)
        } else {
            tf("notification.body.automationFailed", &params)
        },
    );
    event.remote = remote.to_string();
    event.origin = "automation".into();
    event
}

pub fn update_event(kind: &str, version: &str, url: &str, tf: FormatFn<'_>) -> AlertEvent {
    let params = [("version", version), ("kind", kind), ("url", url)];
    let mut event = AlertEvent::new(
        AlertEventKind::Update,
        AlertSeverity::Info,
        tf("notification.title.updateFound", &params),
        tf("notification.body.updateFound", &params),
    );
    event.origin = "updater".into();
    event
}

pub fn engine_event(online: bool, detail: &str, tf: FormatFn<'_>) -> AlertEvent {
    let params = [("error", detail), ("detail", detail)];
    AlertEvent::new(
        AlertEventKind::Engine,
        if online {
            AlertSeverity::Info
        } else {
            AlertSeverity::High
        },
        tf(
            if online {
                "notification.title.engineRestarted"
            } else {
                "notification.title.engineConnectionFailed"
            },
            &params,
        ),
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

    fn tf(key: &str, params: &[(&str, &str)]) -> String {
        let mut out = key.to_string();
        for (_, value) in params {
            out.push(' ');
            out.push_str(value);
        }
        out
    }

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
            parent_job_id: None,
        }
    }

    #[test]
    fn job_events_emit_fail_and_complete() {
        let prev = vec![job(1, "running"), job(2, "running")];
        let curr = vec![job(1, "failed"), job(2, "completed")];
        let events = job_events(&prev, &curr, &tf);
        assert_eq!(events.len(), 2);
        assert!(events.iter().any(|e| e.severity == AlertSeverity::High));
        assert!(events
            .iter()
            .any(|e| e.title.contains("notification.title.jobCompleted")));
    }

    #[test]
    fn mount_and_serve_start_and_stop() {
        let started = mount_events(
            &[],
            &[MountedRemote {
                fs: "drive:".into(),
                mount_point: "/mnt/drive".into(),
            }],
            &tf,
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
                origin: "dashboard".into(),
                profile: "default".into(),
                option_count: 0,
            }],
            &[],
            &tf,
        );
        assert_eq!(stopped[0].kind, AlertEventKind::Serve);
        assert_eq!(stopped[0].severity, AlertSeverity::Warning);
        assert_eq!(stopped[0].remote, "box");
    }

    #[test]
    fn automation_and_update_kinds() {
        let ok = automation_event("nightly", "drive", true, "#12", &tf);
        assert_eq!(ok.kind, AlertEventKind::Automation);
        assert_eq!(ok.severity, AlertSeverity::Info);
        let fail = automation_event("nightly", "drive", false, "offline", &tf);
        assert_eq!(fail.severity, AlertSeverity::High);
        let update = update_event("app", "v2.0.0", "https://example", &tf);
        assert_eq!(update.kind, AlertEventKind::Update);
        let engine = engine_event(false, "rcd exited", &tf);
        assert_eq!(engine.kind, AlertEventKind::Engine);
    }
}
