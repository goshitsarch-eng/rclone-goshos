//! Inline path autocomplete — mirrors Angular `PathSelectionService`.

use crate::path_kind::{expand_user_for_os, is_truly_local_path, is_windows_os};
use crate::rclone::{join_remote_path, split_remote_path, DirEntry};

const MAX_ENTRIES: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutocompleteEntry {
    pub name: String,
    pub is_dir: bool,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutocompleteQuery {
    /// Directory to list (`/` or `C:/` locally, `remote:` for remotes).
    pub list_fs: String,
    /// Relative path inside `list_fs` (empty = root).
    pub list_remote: String,
    pub prefix: String,
    pub is_local: bool,
    /// Path written when the user picks "Up folder".
    pub parent_path: String,
    /// Formatted path of the listed directory (used when joining a child).
    pub listed_path: String,
}

impl AutocompleteQuery {
    pub fn can_go_up(&self) -> bool {
        !self.parent_path.is_empty() && self.parent_path != self.listed_path
    }
}

pub fn parse_autocomplete_query(
    input: &str,
    default_remote: &str,
    engine_os: &str,
) -> AutocompleteQuery {
    let input = input.trim();
    let default_local = default_remote.is_empty() || default_remote == "local";

    if input.is_empty() {
        return if default_local {
            local_root_query(engine_os)
        } else {
            remote_root_query(default_remote)
        };
    }

    if is_local_input(input, engine_os) {
        return local_query(input, engine_os);
    }

    if let Some((remote, rest)) = split_named_remote(input, engine_os) {
        return remote_query(&remote, rest);
    }

    if default_local {
        local_query(input, engine_os)
    } else {
        remote_query(default_remote, input)
    }
}

fn is_local_input(input: &str, engine_os: &str) -> bool {
    is_truly_local_path(input, engine_os)
        || input.starts_with('/')
        || input.starts_with("~/")
        || input == "~"
        || input == "local"
        || input.starts_with("local:")
}

fn split_named_remote<'a>(input: &'a str, engine_os: &str) -> Option<(String, &'a str)> {
    let (remote, rest) = input.split_once(':')?;
    if remote.is_empty() || remote == "/" || remote == "local" {
        return None;
    }
    if is_windows_os(engine_os) && remote.len() == 1 && remote.as_bytes()[0].is_ascii_alphabetic() {
        return None;
    }
    Some((remote.to_string(), rest))
}

fn local_root_query(engine_os: &str) -> AutocompleteQuery {
    if is_windows_os(engine_os) {
        AutocompleteQuery {
            list_fs: String::new(),
            list_remote: String::new(),
            prefix: String::new(),
            is_local: true,
            parent_path: String::new(),
            listed_path: String::new(),
        }
    } else {
        AutocompleteQuery {
            list_fs: "/".into(),
            list_remote: String::new(),
            prefix: String::new(),
            is_local: true,
            parent_path: String::new(),
            listed_path: "/".into(),
        }
    }
}

fn remote_root_query(remote: &str) -> AutocompleteQuery {
    AutocompleteQuery {
        list_fs: format!("{remote}:"),
        list_remote: String::new(),
        prefix: String::new(),
        is_local: false,
        parent_path: String::new(),
        listed_path: format!("{remote}:"),
    }
}

fn local_query(input: &str, engine_os: &str) -> AutocompleteQuery {
    let expanded = expand_user_for_os(input, engine_os);
    let ends_sep = input.ends_with('/') || input.ends_with('\\');
    let normalized = expanded.replace('\\', "/");
    if ends_sep {
        let listed = if normalized.ends_with('/') {
            normalized.clone()
        } else {
            format!("{normalized}/")
        };
        let parent = parent_of_local(&normalized);
        return AutocompleteQuery {
            list_fs: "/".into(),
            list_remote: normalized
                .trim_start_matches('/')
                .trim_end_matches('/')
                .into(),
            prefix: String::new(),
            is_local: true,
            parent_path: parent,
            listed_path: listed,
        };
    }
    let (parent, prefix) = split_last_segment(&normalized);
    let listed = if parent.is_empty() {
        "/".into()
    } else {
        parent.clone()
    };
    AutocompleteQuery {
        list_fs: "/".into(),
        list_remote: listed.trim_start_matches('/').to_string(),
        prefix,
        is_local: true,
        parent_path: parent_of_local(&listed),
        listed_path: listed,
    }
}

fn remote_query(remote: &str, rest: &str) -> AutocompleteQuery {
    let ends_sep = rest.ends_with('/');
    let trimmed = rest.trim_matches('/');
    if rest.is_empty() || ends_sep {
        let listed = if trimmed.is_empty() {
            format!("{remote}:")
        } else {
            format!("{remote}:{trimmed}/")
        };
        return AutocompleteQuery {
            list_fs: format!("{remote}:"),
            list_remote: trimmed.to_string(),
            prefix: String::new(),
            is_local: false,
            parent_path: parent_of_remote(remote, trimmed),
            listed_path: listed,
        };
    }
    let (parent, prefix) = split_last_segment(trimmed);
    let listed = if parent.is_empty() {
        format!("{remote}:")
    } else {
        format!("{remote}:{parent}")
    };
    AutocompleteQuery {
        list_fs: format!("{remote}:"),
        list_remote: parent.clone(),
        prefix,
        is_local: false,
        parent_path: parent_of_remote(remote, &parent),
        listed_path: listed,
    }
}

fn split_last_segment(path: &str) -> (String, String) {
    match path.rsplit_once('/') {
        Some((parent, name)) => (parent.to_string(), name.to_string()),
        None => (String::new(), path.to_string()),
    }
}

fn parent_of_local(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/" {
        return String::new();
    }
    match trimmed.rsplit_once('/') {
        Some(("", _)) => "/".into(),
        Some((parent, _)) if !parent.is_empty() => parent.to_string(),
        _ => "/".into(),
    }
}

fn parent_of_remote(remote: &str, relative: &str) -> String {
    let trimmed = relative.trim_end_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }
    match trimmed.rsplit_once('/') {
        Some((parent, _)) if !parent.is_empty() => format!("{remote}:{parent}"),
        _ => format!("{remote}:"),
    }
}

pub fn join_autocomplete_path(listed_path: &str, name: &str, is_local: bool) -> String {
    if is_local {
        if listed_path.is_empty() || listed_path == "/" {
            if name.starts_with('/') {
                name.to_string()
            } else {
                format!("/{name}")
            }
        } else {
            join_remote_path(listed_path.trim_end_matches('/'), name)
        }
    } else if listed_path.ends_with(':') {
        format!("{listed_path}{name}")
    } else {
        join_remote_path(listed_path.trim_end_matches('/'), name)
    }
}

pub fn filter_entries(entries: &[AutocompleteEntry], prefix: &str) -> Vec<AutocompleteEntry> {
    if prefix.is_empty() {
        return entries.iter().take(MAX_ENTRIES).cloned().collect();
    }
    let prefix = prefix.to_lowercase();
    entries
        .iter()
        .filter(|entry| entry.name.to_lowercase().starts_with(&prefix))
        .take(MAX_ENTRIES)
        .cloned()
        .collect()
}

pub fn entries_from_listing(
    listing: &[DirEntry],
    listed_path: &str,
    is_local: bool,
    prefix: &str,
    folders_only: bool,
) -> Vec<AutocompleteEntry> {
    let mut entries: Vec<AutocompleteEntry> = listing
        .iter()
        .filter(|item| !folders_only || item.is_dir)
        .map(|item| AutocompleteEntry {
            name: item.name.clone(),
            is_dir: item.is_dir,
            path: join_autocomplete_path(listed_path, &item.name, is_local),
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    filter_entries(&entries, prefix)
}

pub fn list_local_entries(
    listed_path: &str,
    prefix: &str,
    folders_only: bool,
) -> Vec<AutocompleteEntry> {
    let dir = if listed_path.is_empty() {
        std::path::Path::new("/")
    } else {
        std::path::Path::new(listed_path)
    };
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut listing = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.is_empty() {
            continue;
        }
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        listing.push(DirEntry {
            name,
            is_dir,
            ..DirEntry::default()
        });
    }
    entries_from_listing(&listing, listed_path, true, prefix, folders_only)
}

/// Split used by `operations/list` for a typed path (`split_remote_path` + local `/`).
pub fn list_target(query: &AutocompleteQuery) -> (String, String) {
    if query.is_local {
        if query.listed_path.is_empty() {
            ("/".into(), String::new())
        } else {
            let (fs, remote) = split_remote_path(&query.listed_path);
            if fs == "local" {
                ("/".into(), remote.trim_start_matches('/').to_string())
            } else {
                (query.list_fs.clone(), query.list_remote.clone())
            }
        }
    } else {
        (query.list_fs.clone(), query.list_remote.clone())
    }
}

pub fn parent_from_path(path: &str) -> String {
    let (remote, rest) = split_remote_path(path);
    if remote == "local" || path.starts_with('/') {
        return parent_of_local(path);
    }
    parent_of_remote(&remote, &rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_defaults_to_remote_or_local_root() {
        let local = parse_autocomplete_query("", "", "linux");
        assert!(local.is_local);
        assert_eq!(local.listed_path, "/");
        let remote = parse_autocomplete_query("", "testdrive", "linux");
        assert!(!remote.is_local);
        assert_eq!(remote.listed_path, "testdrive:");
        assert_eq!(remote.list_fs, "testdrive:");
    }

    #[test]
    fn remote_prefix_lists_parent() {
        let q = parse_autocomplete_query("testdrive:Photos/pre", "testdrive", "linux");
        assert_eq!(q.list_fs, "testdrive:");
        assert_eq!(q.list_remote, "Photos");
        assert_eq!(q.prefix, "pre");
        assert_eq!(q.listed_path, "testdrive:Photos");
        assert_eq!(q.parent_path, "testdrive:");
        assert!(q.can_go_up());
    }

    #[test]
    fn remote_trailing_slash_lists_that_folder() {
        let q = parse_autocomplete_query("testdrive:Photos/", "testdrive", "linux");
        assert_eq!(q.list_remote, "Photos");
        assert_eq!(q.prefix, "");
        assert_eq!(q.listed_path, "testdrive:Photos/");
        assert_eq!(q.parent_path, "testdrive:");
    }

    #[test]
    fn relative_input_uses_default_remote() {
        let q = parse_autocomplete_query("Photos", "testdrive", "linux");
        assert_eq!(q.list_fs, "testdrive:");
        assert_eq!(q.prefix, "Photos");
        assert_eq!(q.listed_path, "testdrive:");
    }

    #[test]
    fn local_unix_path_splits_prefix() {
        let q = parse_autocomplete_query("/tmp/rclone-test/Pho", "", "linux");
        assert!(q.is_local);
        assert_eq!(q.listed_path, "/tmp/rclone-test");
        assert_eq!(q.prefix, "Pho");
        assert_eq!(q.parent_path, "/tmp");
    }

    #[test]
    fn join_remote_and_local_children() {
        assert_eq!(
            join_autocomplete_path("testdrive:", "Photos", false),
            "testdrive:Photos"
        );
        assert_eq!(
            join_autocomplete_path("testdrive:Photos", "img", false),
            "testdrive:Photos/img"
        );
        assert_eq!(join_autocomplete_path("/", "tmp", true), "/tmp");
        assert_eq!(join_autocomplete_path("/tmp", "out", true), "/tmp/out");
    }

    #[test]
    fn filters_and_caps_entries() {
        let listing = vec![
            DirEntry {
                name: "Photos".into(),
                is_dir: true,
                ..DirEntry::default()
            },
            DirEntry {
                name: "preview.pdf".into(),
                is_dir: false,
                ..DirEntry::default()
            },
            DirEntry {
                name: "Docs".into(),
                is_dir: true,
                ..DirEntry::default()
            },
        ];
        let folders = entries_from_listing(&listing, "testdrive:", false, "P", true);
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].name, "Photos");
        assert_eq!(folders[0].path, "testdrive:Photos");
        let both = entries_from_listing(&listing, "testdrive:", false, "", false);
        assert_eq!(both.len(), 3);
        assert!(both[0].is_dir);
    }

    #[test]
    fn lists_real_local_directory() {
        let tmp = std::env::temp_dir().join("rclone-manager-ac-list");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("Photos")).unwrap();
        std::fs::write(tmp.join("readme.txt"), "x").unwrap();
        let listed = tmp.to_string_lossy().into_owned();
        let folders = list_local_entries(&listed, "Ph", true);
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].name, "Photos");
        let all = list_local_entries(&listed, "", false);
        assert!(all.iter().any(|e| e.name == "readme.txt" && !e.is_dir));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn parent_from_remote_path() {
        assert_eq!(parent_from_path("testdrive:Photos/a"), "testdrive:Photos");
    }
}
