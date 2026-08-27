//! Desktop notifications with click-through, shared by GTK UI and store alerts.
//!
//! Kept out of `platform` so `store` can emit OS toasts without a
//! `store` → `platform` → `tray_menu` → `store` cycle.

use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationTarget {
    ShowWindow,
    Job(u64),
    Alerts,
    Repair,
}

static CLICKS: Mutex<Vec<NotificationTarget>> = Mutex::new(Vec::new());

pub fn push_notification_click(target: NotificationTarget) {
    if let Ok(mut queue) = CLICKS.lock() {
        queue.push(target);
    }
}

pub fn drain_notification_clicks() -> Vec<NotificationTarget> {
    CLICKS
        .lock()
        .map(|mut queue| queue.drain(..).collect())
        .unwrap_or_default()
}

pub fn show_os_notification(title: &str, body: &str) -> bool {
    show_os_notification_target(title, body, NotificationTarget::ShowWindow)
}

pub fn show_os_notification_target(title: &str, body: &str, target: NotificationTarget) -> bool {
    let title = title.to_string();
    let body = body.to_string();
    std::thread::spawn(move || {
        let mut notification = notify_rust::Notification::new();
        notification
            .appname("Rclone Manager")
            .summary(&title)
            .body(&body)
            .icon("folder-remote")
            .action("default", "Open")
            .action("open", "Open");
        if let Ok(handle) = notification.show() {
            handle.wait_for_action(|action| {
                if action != "__closed" {
                    push_notification_click(target.clone());
                }
            });
        }
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_queue_drains_fifo() {
        let _ = drain_notification_clicks();
        push_notification_click(NotificationTarget::Job(42));
        push_notification_click(NotificationTarget::Alerts);
        assert_eq!(
            drain_notification_clicks(),
            vec![NotificationTarget::Job(42), NotificationTarget::Alerts]
        );
        assert!(drain_notification_clicks().is_empty());
    }
}
