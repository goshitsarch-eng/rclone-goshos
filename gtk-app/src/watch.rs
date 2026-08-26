//! Filesystem watch hub — `notify` events plus dirty-path matching for automations.

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct WatchHub {
    dirty: Arc<Mutex<HashSet<String>>>,
    watched: HashSet<String>,
    watcher: Option<Arc<Mutex<RecommendedWatcher>>>,
}

impl WatchHub {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ensure_paths(&mut self, paths: &[String]) {
        if self.watcher.is_none() {
            let dirty = self.dirty.clone();
            if let Ok(watcher) = RecommendedWatcher::new(
                move |res: Result<Event, notify::Error>| {
                    if let Ok(event) = res {
                        if let Ok(mut set) = dirty.lock() {
                            for path in event.paths {
                                set.insert(path.to_string_lossy().into_owned());
                            }
                        }
                    }
                },
                Config::default(),
            ) {
                self.watcher = Some(Arc::new(Mutex::new(watcher)));
            }
        }
        let Some(watcher) = &self.watcher else {
            return;
        };
        let Ok(mut guard) = watcher.lock() else {
            return;
        };
        for path in paths {
            if path.is_empty() || !Path::new(path).exists() {
                continue;
            }
            if self.watched.insert(path.clone()) {
                let _ = guard.watch(Path::new(path), RecursiveMode::Recursive);
            }
        }
    }

    pub fn consume_dirty(&self) -> HashSet<String> {
        self.dirty
            .lock()
            .map(|mut set| std::mem::take(&mut *set))
            .unwrap_or_default()
    }
}

/// True when any watched source is the dirty path or an ancestor/descendant of it.
pub fn dirty_matches(sources: &[String], dirty: &HashSet<String>) -> bool {
    !dirty_for_sources(sources, dirty).is_empty()
}

/// Dirty paths that overlap any automation source (used for scoped watch jobs).
pub fn dirty_for_sources(sources: &[String], dirty: &HashSet<String>) -> HashSet<String> {
    dirty
        .iter()
        .filter(|path| {
            sources
                .iter()
                .any(|src| !src.is_empty() && paths_overlap(src, path))
        })
        .cloned()
        .collect()
}

pub fn paths_overlap(a: &str, b: &str) -> bool {
    let a = a.trim_end_matches('/');
    let b = b.trim_end_matches('/');
    if a.is_empty() || b.is_empty() {
        return false;
    }
    a == b || b.starts_with(&format!("{a}/")) || a.starts_with(&format!("{b}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_detects_child_and_parent() {
        assert!(paths_overlap("/tmp/docs", "/tmp/docs/a.txt"));
        assert!(paths_overlap("/tmp/docs/a.txt", "/tmp/docs"));
        assert!(paths_overlap("/tmp/docs", "/tmp/docs"));
        assert!(!paths_overlap("/tmp/docs", "/tmp/other"));
        assert!(!paths_overlap("", "/tmp/docs"));
    }

    #[test]
    fn dirty_matches_any_source() {
        let mut dirty = HashSet::new();
        dirty.insert("/home/me/Photos/IMG_1.jpg".into());
        assert!(dirty_matches(&["/home/me/Photos".into()], &dirty));
        assert!(!dirty_matches(&["drive:Photos".into()], &dirty));
        assert!(!dirty_matches(&["".into()], &dirty));
        let scoped = dirty_for_sources(&["/home/me/Photos".into()], &dirty);
        assert_eq!(scoped.len(), 1);
        assert!(scoped.contains("/home/me/Photos/IMG_1.jpg"));
    }
}
