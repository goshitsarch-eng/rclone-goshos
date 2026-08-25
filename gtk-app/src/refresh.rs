//! Adaptive runtime refresh helpers.

use std::time::Duration;

pub const BUSY_POLL: Duration = Duration::from_millis(400);
pub const IDLE_POLL: Duration = Duration::from_secs(3);
pub const HIDDEN_POLL: Duration = Duration::from_secs(15);
pub const IDLE_TICKS: u32 = 7;

pub fn runtime_busy(running_jobs: bool, mount_count: usize, serve_count: usize) -> bool {
    running_jobs || mount_count > 0 || serve_count > 0
}

pub fn poll_interval(busy: bool) -> Duration {
    poll_interval_for(busy, true)
}

pub fn poll_interval_for(busy: bool, visible: bool) -> Duration {
    if !visible && !busy {
        HIDDEN_POLL
    } else if busy {
        BUSY_POLL
    } else {
        IDLE_POLL
    }
}

pub fn idle_ticks_for(target: Duration) -> u32 {
    let busy_ms = BUSY_POLL.as_millis().max(1);
    (target.as_millis() / busy_ms).max(1) as u32
}

pub fn should_refresh(tick: u32, busy: bool, idle_every: u32) -> bool {
    let every = idle_every.max(1);
    busy || tick % every == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_when_jobs_or_mounts_or_serves() {
        assert!(runtime_busy(true, 0, 0));
        assert!(runtime_busy(false, 1, 0));
        assert!(runtime_busy(false, 0, 2));
        assert!(!runtime_busy(false, 0, 0));
    }

    #[test]
    fn intervals_match_busy_state() {
        assert_eq!(poll_interval(true), BUSY_POLL);
        assert_eq!(poll_interval(false), IDLE_POLL);
        assert_eq!(poll_interval_for(false, false), HIDDEN_POLL);
        assert_eq!(poll_interval_for(true, false), BUSY_POLL);
        assert_eq!(idle_ticks_for(HIDDEN_POLL) > IDLE_TICKS, true);
    }

    #[test]
    fn idle_refresh_every_nth_tick() {
        assert_eq!(idle_ticks_for(IDLE_POLL), IDLE_TICKS);
        assert!(should_refresh(0, false, 8));
        assert!(!should_refresh(1, false, 8));
        assert!(should_refresh(8, false, 8));
        assert!(should_refresh(3, true, 8));
    }
}
