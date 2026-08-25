//! Embedded Nautilus file-picker constraints — mirrors Angular `FilePickerConfig`.

use crate::rclone::split_remote_path;
use std::rc::Rc;

pub struct PickerRequest {
    pub config: FilePickerConfig,
    pub on_pick: Rc<dyn Fn(PickerResult)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerMode {
    Local,
    Remote,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerSelection {
    Files,
    Folders,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePickerConfig {
    pub mode: PickerMode,
    pub selection: PickerSelection,
    pub multi: bool,
    pub allowed_remotes: Vec<String>,
    pub allowed_extensions: Vec<String>,
    pub initial_location: Option<String>,
}

impl Default for FilePickerConfig {
    fn default() -> Self {
        Self {
            mode: PickerMode::Both,
            selection: PickerSelection::Both,
            multi: false,
            allowed_remotes: vec![],
            allowed_extensions: vec![],
            initial_location: None,
        }
    }
}

impl FilePickerConfig {
    pub fn folders() -> Self {
        Self {
            selection: PickerSelection::Folders,
            ..Self::default()
        }
    }

    pub fn local_folders() -> Self {
        Self {
            mode: PickerMode::Local,
            selection: PickerSelection::Folders,
            ..Self::default()
        }
    }

    pub fn remote_folders(current: &str) -> Self {
        Self {
            mode: PickerMode::Remote,
            selection: PickerSelection::Folders,
            allowed_remotes: if current.is_empty() || current == "local" {
                vec![]
            } else {
                vec![current.to_string()]
            },
            initial_location: if current.is_empty() || current == "local" {
                None
            } else {
                Some(format!("{current}:"))
            },
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PickerResult {
    pub remote: String,
    pub path: String,
    pub is_dir: bool,
    pub cancelled: bool,
}

impl PickerResult {
    pub fn formatted_path(&self) -> String {
        format_picker_path(&self.remote, &self.path)
    }
}

pub fn format_picker_path(remote: &str, path: &str) -> String {
    if remote == "local" || remote.is_empty() {
        if path.is_empty() {
            "/".into()
        } else {
            path.to_string()
        }
    } else if path.is_empty() {
        format!("{remote}:")
    } else {
        format!("{remote}:{path}")
    }
}

pub fn is_location_allowed(loc: &str, cfg: &FilePickerConfig) -> bool {
    let (remote, path) = split_remote_path(loc);
    let is_local = remote == "local" || loc.starts_with('/') || loc.starts_with('~');
    match cfg.mode {
        PickerMode::Local => is_local,
        PickerMode::Remote => {
            if is_local {
                return false;
            }
            if cfg.allowed_remotes.is_empty() {
                return true;
            }
            cfg.allowed_remotes.iter().any(|name| name == &remote)
        }
        PickerMode::Both => {
            if is_local {
                return true;
            }
            if cfg.allowed_remotes.is_empty() {
                return true;
            }
            cfg.allowed_remotes.iter().any(|name| name == &remote)
                || (remote.is_empty() && path.is_empty())
        }
    }
}

pub fn is_entry_allowed(name: &str, is_dir: bool, cfg: &FilePickerConfig) -> bool {
    match cfg.selection {
        PickerSelection::Folders => is_dir,
        PickerSelection::Files => {
            if is_dir {
                return false;
            }
            extension_allowed(name, cfg)
        }
        PickerSelection::Both => {
            if is_dir {
                true
            } else {
                extension_allowed(name, cfg)
            }
        }
    }
}

fn extension_allowed(name: &str, cfg: &FilePickerConfig) -> bool {
    if cfg.allowed_extensions.is_empty() {
        return true;
    }
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    cfg.allowed_extensions
        .iter()
        .any(|allowed| allowed.trim_start_matches('.').eq_ignore_ascii_case(&ext))
}

pub fn can_confirm_selection(
    selected_dirs: usize,
    selected_files: usize,
    cfg: &FilePickerConfig,
) -> bool {
    match cfg.selection {
        PickerSelection::Folders => selected_dirs > 0 || selected_files == 0,
        PickerSelection::Files => selected_files > 0,
        PickerSelection::Both => selected_dirs + selected_files > 0 || true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_rules_match_angular() {
        let local = FilePickerConfig {
            mode: PickerMode::Local,
            ..FilePickerConfig::default()
        };
        assert!(is_location_allowed("/home/ada", &local));
        assert!(!is_location_allowed("drive:Photos", &local));
        let remote = FilePickerConfig {
            mode: PickerMode::Remote,
            allowed_remotes: vec!["drive".into()],
            ..FilePickerConfig::default()
        };
        assert!(is_location_allowed("drive:Photos", &remote));
        assert!(!is_location_allowed("photos:x", &remote));
        assert!(!is_location_allowed("/tmp", &remote));
        let both = FilePickerConfig {
            mode: PickerMode::Both,
            allowed_remotes: vec!["drive".into()],
            ..FilePickerConfig::default()
        };
        assert!(is_location_allowed("/tmp", &both));
        assert!(is_location_allowed("drive:", &both));
        assert!(!is_location_allowed("other:x", &both));
    }

    #[test]
    fn entry_rules_and_extensions() {
        let folders = FilePickerConfig::folders();
        assert!(is_entry_allowed("docs", true, &folders));
        assert!(!is_entry_allowed("a.txt", false, &folders));
        let files = FilePickerConfig {
            selection: PickerSelection::Files,
            allowed_extensions: vec!["txt".into(), ".md".into()],
            ..FilePickerConfig::default()
        };
        assert!(is_entry_allowed("note.md", false, &files));
        assert!(!is_entry_allowed("pic.png", false, &files));
        assert!(!is_entry_allowed("docs", true, &files));
    }

    #[test]
    fn formats_picker_paths() {
        assert_eq!(format_picker_path("local", "/tmp/out"), "/tmp/out");
        assert_eq!(format_picker_path("drive", ""), "drive:");
        assert_eq!(format_picker_path("drive", "Photos"), "drive:Photos");
        assert_eq!(
            PickerResult {
                remote: "drive".into(),
                path: "a".into(),
                is_dir: true,
                cancelled: false,
            }
            .formatted_path(),
            "drive:a"
        );
    }
}
