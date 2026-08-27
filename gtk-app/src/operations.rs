//! Operation registry — mirrors `src/app/shared/types/operation-registry.ts`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OperationType {
    Mount,
    Sync,
    Copy,
    Move,
    Bisync,
    Serve,
    Check,
    Delete,
    Copyurl,
    Archivecreate,
    Cryptcheck,
}

impl OperationType {
    pub const ALL: [OperationType; 11] = [
        Self::Mount,
        Self::Sync,
        Self::Copy,
        Self::Move,
        Self::Bisync,
        Self::Serve,
        Self::Check,
        Self::Delete,
        Self::Copyurl,
        Self::Archivecreate,
        Self::Cryptcheck,
    ];

    pub const PRIMARY_SYNC: [OperationType; 4] = [Self::Sync, Self::Copy, Self::Move, Self::Bisync];

    pub const MORE_SYNC: [OperationType; 5] = [
        Self::Check,
        Self::Delete,
        Self::Copyurl,
        Self::Archivecreate,
        Self::Cryptcheck,
    ];

    /// Same order as Angular `SYNC_TYPES` (registry `isSyncType` keys).
    pub const SYNC_TYPES: [OperationType; 9] = [
        Self::Sync,
        Self::Copy,
        Self::Move,
        Self::Bisync,
        Self::Check,
        Self::Delete,
        Self::Copyurl,
        Self::Archivecreate,
        Self::Cryptcheck,
    ];

    pub const SERVE_TYPES: [&'static str; 8] = [
        "http", "webdav", "ftp", "sftp", "nfs", "dlna", "restic", "s3",
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mount => "mount",
            Self::Sync => "sync",
            Self::Copy => "copy",
            Self::Move => "move",
            Self::Bisync => "bisync",
            Self::Serve => "serve",
            Self::Check => "check",
            Self::Delete => "delete",
            Self::Copyurl => "copyurl",
            Self::Archivecreate => "archivecreate",
            Self::Cryptcheck => "cryptcheck",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "mount" => Some(Self::Mount),
            "sync" => Some(Self::Sync),
            "copy" => Some(Self::Copy),
            "move" => Some(Self::Move),
            "bisync" => Some(Self::Bisync),
            "serve" => Some(Self::Serve),
            "check" => Some(Self::Check),
            "delete" => Some(Self::Delete),
            "copyurl" => Some(Self::Copyurl),
            "archivecreate" | "archive" => Some(Self::Archivecreate),
            "cryptcheck" => Some(Self::Cryptcheck),
            _ => None,
        }
    }

    pub fn api_label(self) -> &'static str {
        match self {
            Self::Mount => "Mount",
            Self::Sync => "Sync",
            Self::Copy => "Copy",
            Self::Move => "Move",
            Self::Bisync => "Bisync",
            Self::Serve => "Serve",
            Self::Check => "Check",
            Self::Delete => "Delete",
            Self::Copyurl => "Copyurl",
            Self::Archivecreate => "Archivecreate",
            Self::Cryptcheck => "Cryptcheck",
        }
    }

    pub fn icon_name(self) -> &'static str {
        match self {
            Self::Mount => "drive-harddisk-symbolic",
            Self::Sync => "view-refresh-symbolic",
            Self::Copy => "edit-copy-symbolic",
            Self::Move => "go-jump-symbolic",
            Self::Bisync => "media-playlist-repeat-symbolic",
            Self::Serve => "network-server-symbolic",
            Self::Check => "system-search-symbolic",
            Self::Delete => "user-trash-symbolic",
            Self::Copyurl => "insert-link-symbolic",
            Self::Archivecreate => "package-x-generic-symbolic",
            Self::Cryptcheck => "security-high-symbolic",
        }
    }

    pub fn action_label_key(self) -> &'static str {
        match self {
            Self::Mount => "actions.mount",
            Self::Sync => "actions.sync",
            Self::Copy => "actions.copy",
            Self::Move => "actions.move",
            Self::Bisync => "actions.bisync",
            Self::Serve => "actions.serve",
            Self::Check => "actions.check",
            Self::Delete => "actions.delete",
            Self::Copyurl => "actions.copyurl",
            Self::Archivecreate => "actions.archivecreate",
            Self::Cryptcheck => "actions.cryptcheck",
        }
    }

    /// Compact remote-card action i18n key (`overviews.remoteCard.actions.*`).
    pub fn remote_card_action_key(self, active: bool) -> String {
        match self {
            Self::Mount => {
                if active {
                    "overviews.remoteCard.actions.unmount".into()
                } else {
                    "overviews.remoteCard.actions.mount".into()
                }
            }
            other => {
                let verb = if active { "stop" } else { "start" };
                let name = other.as_str();
                let capitalized = format!("{}{}", name[..1].to_ascii_uppercase(), &name[1..]);
                format!("overviews.remoteCard.actions.{verb}{capitalized}")
            }
        }
    }

    pub fn is_sync_type(self) -> bool {
        !matches!(self, Self::Mount | Self::Serve)
    }

    pub fn is_primary(self) -> bool {
        true
    }

    pub fn is_browsable(self) -> bool {
        matches!(
            self,
            Self::Mount
                | Self::Sync
                | Self::Copy
                | Self::Move
                | Self::Bisync
                | Self::Check
                | Self::Cryptcheck
        )
    }

    pub fn is_automatable(self) -> bool {
        self.is_sync_type()
    }

    pub fn supports_vfs(self) -> bool {
        matches!(self, Self::Mount | Self::Serve)
    }

    pub fn supports_multi_source(self) -> bool {
        matches!(
            self,
            Self::Sync
                | Self::Copy
                | Self::Move
                | Self::Delete
                | Self::Copyurl
                | Self::Check
                | Self::Cryptcheck
        )
    }

    pub fn supports_profiles(self) -> bool {
        true
    }

    pub fn config_key(self) -> &'static str {
        match self {
            Self::Mount => "mountConfigs",
            Self::Sync => "syncConfigs",
            Self::Copy => "copyConfigs",
            Self::Move => "moveConfigs",
            Self::Bisync => "bisyncConfigs",
            Self::Serve => "serveConfigs",
            Self::Check => "checkConfigs",
            Self::Delete => "deleteConfigs",
            Self::Copyurl => "copyurlConfigs",
            Self::Archivecreate => "archivecreateConfigs",
            Self::Cryptcheck => "cryptcheckConfigs",
        }
    }

    pub fn rc_job_endpoint(self) -> Option<&'static str> {
        match self {
            Self::Sync => Some("sync/sync"),
            Self::Copy => Some("sync/copy"),
            Self::Move => Some("sync/move"),
            Self::Bisync => Some("sync/bisync"),
            Self::Check => Some("operations/check"),
            Self::Delete => Some("operations/delete"),
            Self::Copyurl => Some("operations/copyurl"),
            Self::Cryptcheck => Some("operations/cryptcheck"),
            Self::Archivecreate => Some("operations/archive"),
            Self::Mount | Self::Serve => None,
        }
    }
}

/// Merge live rclone type lists with the built-in fallback, keeping first-seen order.
pub fn merge_known_types(live: &[String], fallback: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let push = |out: &mut Vec<String>, item: &str| {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            return;
        }
        if !out
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(trimmed))
        {
            out.push(trimmed.to_string());
        }
    };
    for item in live {
        push(&mut out, item);
    }
    for item in fallback {
        push(&mut out, item);
    }
    if out.is_empty() {
        out.extend(fallback.iter().map(|s| (*s).to_string()));
    }
    out
}

pub fn serve_types_or_default(live: &[String]) -> Vec<String> {
    merge_known_types(live, &OperationType::SERVE_TYPES)
}

pub fn mount_types_or_default(live: &[String]) -> Vec<String> {
    merge_known_types(live, &["mount"])
}

pub fn combo_names(items: &[String]) -> Vec<&str> {
    items.iter().map(String::as_str).collect()
}

pub fn selected_or<'a>(items: &'a [String], idx: u32, fallback: &'a str) -> &'a str {
    items
        .get(idx as usize)
        .map(String::as_str)
        .unwrap_or(fallback)
}

impl std::fmt::Display for OperationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MainView {
    MainMenu,
    Nautilus,
    Flow,
}

impl MainView {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MainMenu => "main_menu",
            Self::Nautilus => "nautilus",
            Self::Flow => "flow",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "nautilus" => Self::Nautilus,
            "flow" => Self::Flow,
            _ => Self::MainMenu,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppTab {
    General,
    Mount,
    Operations,
    Serve,
}

impl AppTab {
    pub const ALL: [AppTab; 4] = [Self::General, Self::Mount, Self::Operations, Self::Serve];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Mount => "mount",
            Self::Operations => "operations",
            Self::Serve => "serve",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "general" => Some(Self::General),
            "mount" => Some(Self::Mount),
            "operations" => Some(Self::Operations),
            "serve" => Some(Self::Serve),
            _ => None,
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            Self::General => "tabs.general",
            Self::Mount => "tabs.mount",
            Self::Operations => "tabs.operations",
            Self::Serve => "tabs.serve",
        }
    }

    pub fn icon_name(self) -> &'static str {
        match self {
            Self::General => "go-home-symbolic",
            Self::Mount => "drive-harddisk-symbolic",
            Self::Operations => "media-playlist-consecutive-symbolic",
            Self::Serve => "network-server-symbolic",
        }
    }

    pub fn includes_operation(self, op: OperationType) -> bool {
        match self {
            Self::General => true,
            Self::Mount => op == OperationType::Mount,
            Self::Serve => op == OperationType::Serve,
            Self::Operations => !matches!(op, OperationType::Mount | OperationType::Serve),
        }
    }

    /// Configuration / Monitoring lists — Angular `currentOpType()` scoping.
    pub fn lists_profile_op(self, detail_op: OperationType, op: OperationType) -> bool {
        self == Self::General || op == detail_op
    }

    pub fn default_operation(self) -> OperationType {
        match self {
            Self::Mount => OperationType::Mount,
            Self::Serve => OperationType::Serve,
            Self::Operations | Self::General => OperationType::Sync,
        }
    }

    pub fn remote_is_active(self, mounted: bool, serving: bool, job_active: bool) -> bool {
        match self {
            Self::General => mounted || serving || job_active,
            Self::Mount => mounted,
            Self::Serve => serving,
            Self::Operations => job_active,
        }
    }

    pub fn active_section_fallback(self) -> &'static str {
        match self {
            Self::General => "Active",
            Self::Mount => "Mounted",
            Self::Operations => "Running",
            Self::Serve => "Serving",
        }
    }

    pub fn idle_section_fallback(self) -> &'static str {
        match self {
            Self::General => "Available",
            Self::Mount => "Not mounted",
            Self::Operations => "Idle",
            Self::Serve => "Not serving",
        }
    }

    pub fn active_section_key(self) -> &'static str {
        match self {
            Self::General => "generalOverview.active",
            Self::Mount => "mountOverview.active",
            Self::Operations => "operationsOverview.active",
            Self::Serve => "serveOverview.active",
        }
    }

    pub fn mode_defaults(self) -> &'static [OperationType] {
        match self {
            Self::General => &[
                OperationType::Mount,
                OperationType::Sync,
                OperationType::Bisync,
            ],
            Self::Operations => &[
                OperationType::Sync,
                OperationType::Bisync,
                OperationType::Copy,
            ],
            Self::Mount => &[OperationType::Mount],
            Self::Serve => &[OperationType::Serve],
        }
    }

    /// Compact remote-card actions — same source/limit rules as Angular `primaryActionsFor`.
    pub fn compact_primary_ops(
        self,
        primary: &[String],
        sync: &[String],
        limit: usize,
    ) -> Vec<OperationType> {
        let parsed = |items: &[String]| -> Vec<OperationType> {
            items
                .iter()
                .filter_map(|s| OperationType::parse(s))
                .collect()
        };
        let source = match self {
            Self::Operations => {
                let ops = parsed(sync);
                if ops.is_empty() {
                    self.mode_defaults().to_vec()
                } else {
                    ops.into_iter().filter(|op| op.is_sync_type()).collect()
                }
            }
            Self::Mount | Self::Serve => self.mode_defaults().to_vec(),
            Self::General => {
                let ops = parsed(primary);
                if ops.is_empty() {
                    self.mode_defaults().to_vec()
                } else {
                    ops
                }
            }
        };
        let include_mount = !matches!(self, Self::Operations);
        let mut seen = Vec::new();
        for op in source {
            if !include_mount && op == OperationType::Mount {
                continue;
            }
            if !seen.contains(&op) {
                seen.push(op);
            }
            if seen.len() >= limit {
                break;
            }
        }
        seen
    }

    /// Operations-tab primary toggles — Angular `primarySyncOps`.
    pub fn primary_sync_ops(sync_actions: &[String]) -> Vec<OperationType> {
        let custom: Vec<OperationType> = sync_actions
            .iter()
            .filter_map(|s| OperationType::parse(s))
            .filter(|op| op.is_sync_type())
            .collect();
        if custom.is_empty() {
            Self::Operations.mode_defaults().to_vec()
        } else {
            custom.into_iter().take(3).collect()
        }
    }

    /// Overflow operations for the More menu — Angular `moreSyncOps`.
    pub fn more_sync_ops(primary: &[OperationType]) -> Vec<OperationType> {
        OperationType::SYNC_TYPES
            .iter()
            .copied()
            .filter(|op| !primary.contains(op))
            .collect()
    }

    pub fn idle_section_key(self) -> &'static str {
        match self {
            Self::General => "generalOverview.inactive",
            Self::Mount => "mountOverview.inactive",
            Self::Operations => "operationsOverview.inactive",
            Self::Serve => "serveOverview.inactive",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileTypeCategory {
    Directory,
    Image,
    Video,
    Audio,
    Pdf,
    Text,
    Archive,
    Binary,
}

impl FileTypeCategory {
    pub fn from_mime(mime: &str) -> Option<Self> {
        let normalized = mime
            .to_ascii_lowercase()
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if normalized.is_empty() {
            return None;
        }
        if normalized == "application/pdf" {
            return Some(Self::Pdf);
        }
        if matches!(
            normalized.as_str(),
            "application/zip"
                | "application/x-tar"
                | "application/gzip"
                | "application/x-7z-compressed"
                | "application/x-rar-compressed"
                | "application/x-bzip2"
                | "application/x-xz"
                | "application/x-iso9660-image"
        ) {
            return Some(Self::Archive);
        }
        Some(match normalized.split('/').next().unwrap_or_default() {
            "image" => Self::Image,
            "video" => Self::Video,
            "audio" => Self::Audio,
            "text" => Self::Text,
            _ => return None,
        })
    }

    pub fn from_entry(name: &str, is_dir: bool, mime: &str) -> Self {
        if is_dir {
            return Self::Directory;
        }
        Self::from_mime(mime).unwrap_or_else(|| Self::from_name(name, false))
    }

    pub fn from_name(name: &str, is_dir: bool) -> Self {
        if is_dir {
            return Self::Directory;
        }
        let ext = std::path::Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "avif" | "jxl" | "tif"
            | "tiff" | "ico" => Self::Image,
            "mp4" | "mkv" | "webm" | "avi" | "mov" | "m4v" | "ogv" => Self::Video,
            "mp3" | "flac" | "ogg" | "wav" | "m4a" | "aac" | "opus" | "wma" => Self::Audio,
            "pdf" => Self::Pdf,
            "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "iso" => Self::Archive,
            "txt" | "md" | "json" | "toml" | "yml" | "yaml" | "xml" | "csv" | "rs" | "ts"
            | "js" | "html" | "css" | "py" | "go" | "c" | "h" | "cpp" | "sh" | "log" | "ini"
            | "conf" | "cfg" => Self::Text,
            _ => Self::Binary,
        }
    }

    pub fn icon_name(self) -> &'static str {
        match self {
            Self::Directory => "folder-symbolic",
            Self::Image => "image-x-generic-symbolic",
            Self::Video => "video-x-generic-symbolic",
            Self::Audio => "audio-x-generic-symbolic",
            Self::Pdf => "x-office-document-symbolic",
            Self::Text => "text-x-generic-symbolic",
            Self::Archive => "package-x-generic-symbolic",
            Self::Binary => "application-x-executable-symbolic",
        }
    }

    pub fn matches_filter(self, filter: &str) -> bool {
        match filter.trim().to_ascii_lowercase().as_str() {
            "" | "all" => true,
            "folder" | "folders" | "directory" => self == Self::Directory,
            "image" | "images" => self == Self::Image,
            "video" | "videos" => self == Self::Video,
            "audio" => self == Self::Audio,
            "document" | "documents" => matches!(self, Self::Pdf | Self::Text),
            "archive" | "archives" => self == Self::Archive,
            _ => true,
        }
    }

    /// Image/video/audio can play from an `--rc-serve` HTTP URL without a full download.
    pub fn can_stream_preview(self) -> bool {
        matches!(self, Self::Image | Self::Video | Self::Audio)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_card_action_keys_match_angular() {
        assert_eq!(
            OperationType::Mount.remote_card_action_key(false),
            "overviews.remoteCard.actions.mount"
        );
        assert_eq!(
            OperationType::Mount.remote_card_action_key(true),
            "overviews.remoteCard.actions.unmount"
        );
        assert_eq!(
            OperationType::Copy.remote_card_action_key(false),
            "overviews.remoteCard.actions.startCopy"
        );
        assert_eq!(
            OperationType::Copy.remote_card_action_key(true),
            "overviews.remoteCard.actions.stopCopy"
        );
        assert_eq!(
            OperationType::Copyurl.remote_card_action_key(false),
            "overviews.remoteCard.actions.startCopyurl"
        );
        assert_eq!(
            OperationType::Archivecreate.remote_card_action_key(true),
            "overviews.remoteCard.actions.stopArchivecreate"
        );
        assert_eq!(
            OperationType::Cryptcheck.remote_card_action_key(false),
            "overviews.remoteCard.actions.startCryptcheck"
        );
        for op in OperationType::ALL {
            if op == OperationType::Mount {
                continue;
            }
            let start = op.remote_card_action_key(false);
            let stop = op.remote_card_action_key(true);
            assert!(start.starts_with("overviews.remoteCard.actions.start"));
            assert!(stop.starts_with("overviews.remoteCard.actions.stop"));
            assert_ne!(start, stop);
        }
    }

    #[test]
    fn parses_all_operation_keys() {
        for op in OperationType::ALL {
            assert_eq!(OperationType::parse(op.as_str()), Some(op));
        }
        assert_eq!(
            OperationType::parse("archive"),
            Some(OperationType::Archivecreate)
        );
        assert_eq!(OperationType::parse("nope"), None);
    }

    #[test]
    fn classification_matches_registry() {
        assert!(OperationType::Sync.is_sync_type());
        assert!(!OperationType::Mount.is_sync_type());
        assert!(OperationType::Mount.supports_vfs());
        assert!(OperationType::Serve.supports_vfs());
        assert!(!OperationType::Copy.supports_vfs());
        assert!(OperationType::Copy.is_automatable());
        assert!(!OperationType::Mount.is_automatable());
        assert!(!OperationType::Delete.is_browsable());
        assert!(OperationType::Cryptcheck.is_browsable());
        assert_eq!(
            OperationType::Cryptcheck.rc_job_endpoint(),
            Some("operations/cryptcheck")
        );
        assert_eq!(
            OperationType::Check.rc_job_endpoint(),
            Some("operations/check")
        );
        assert_eq!(
            OperationType::Archivecreate.rc_job_endpoint(),
            Some("operations/archive")
        );
        assert_eq!(OperationType::Mount.config_key(), "mountConfigs");
        assert_eq!(OperationType::SERVE_TYPES.len(), 8);
        assert!(OperationType::Sync.supports_multi_source());
        assert!(!OperationType::Mount.supports_multi_source());
    }

    #[test]
    fn file_type_categories() {
        assert_eq!(
            FileTypeCategory::from_name("photos", true),
            FileTypeCategory::Directory
        );
        assert_eq!(
            FileTypeCategory::from_name("a.PNG", false),
            FileTypeCategory::Image
        );
        assert_eq!(
            FileTypeCategory::from_name("clip.mkv", false),
            FileTypeCategory::Video
        );
        assert_eq!(
            FileTypeCategory::from_name("song.flac", false),
            FileTypeCategory::Audio
        );
        assert_eq!(
            FileTypeCategory::from_name("doc.pdf", false),
            FileTypeCategory::Pdf
        );
        assert_eq!(
            FileTypeCategory::from_name("notes.md", false),
            FileTypeCategory::Text
        );
        assert_eq!(
            FileTypeCategory::from_name("pack.zip", false),
            FileTypeCategory::Archive
        );
        assert_eq!(
            FileTypeCategory::from_name("blob.bin", false),
            FileTypeCategory::Binary
        );
        assert!(FileTypeCategory::Image.matches_filter("images"));
        assert!(!FileTypeCategory::Video.matches_filter("images"));
        assert!(FileTypeCategory::Pdf.matches_filter("documents"));
        assert!(FileTypeCategory::Text.matches_filter("all"));
        assert_eq!(
            FileTypeCategory::from_mime("image/jpeg; charset=binary"),
            Some(FileTypeCategory::Image)
        );
        assert_eq!(
            FileTypeCategory::from_mime("application/pdf"),
            Some(FileTypeCategory::Pdf)
        );
        assert_eq!(
            FileTypeCategory::from_entry("blob", false, "audio/mpeg"),
            FileTypeCategory::Audio
        );
        assert_eq!(
            FileTypeCategory::from_entry("notes.md", false, ""),
            FileTypeCategory::Text
        );
        assert_eq!(FileTypeCategory::from_mime(""), None);
        assert!(FileTypeCategory::Video.can_stream_preview());
        assert!(FileTypeCategory::Audio.can_stream_preview());
        assert!(FileTypeCategory::Image.can_stream_preview());
        assert!(!FileTypeCategory::Pdf.can_stream_preview());
        assert!(!FileTypeCategory::Text.can_stream_preview());
    }

    #[test]
    fn main_view_and_tabs() {
        assert_eq!(MainView::parse("flow"), MainView::Flow);
        assert_eq!(MainView::parse("unknown"), MainView::MainMenu);
        assert_eq!(AppTab::ALL.len(), 4);
        assert_eq!(AppTab::parse("operations"), Some(AppTab::Operations));
        assert_eq!(AppTab::parse("serve"), Some(AppTab::Serve));
        assert_eq!(AppTab::parse("nope"), None);
        assert!(AppTab::Mount.includes_operation(OperationType::Mount));
        assert!(!AppTab::Mount.includes_operation(OperationType::Sync));
        assert!(AppTab::Operations.includes_operation(OperationType::Bisync));
        assert!(!AppTab::Operations.includes_operation(OperationType::Serve));
        assert!(AppTab::General.lists_profile_op(OperationType::Check, OperationType::Copy));
        assert!(AppTab::Operations.lists_profile_op(OperationType::Check, OperationType::Check));
        assert!(!AppTab::Operations.lists_profile_op(OperationType::Check, OperationType::Copy));
        assert!(AppTab::Mount.lists_profile_op(OperationType::Mount, OperationType::Mount));
        assert!(!AppTab::Mount.lists_profile_op(OperationType::Mount, OperationType::Sync));
        assert_eq!(AppTab::Serve.default_operation(), OperationType::Serve);
        assert!(AppTab::Mount.remote_is_active(true, false, false));
        assert!(!AppTab::Serve.remote_is_active(true, false, true));
        assert!(AppTab::Operations.remote_is_active(false, false, true));
        assert_eq!(
            AppTab::General.mode_defaults(),
            &[
                OperationType::Mount,
                OperationType::Sync,
                OperationType::Bisync
            ]
        );
        assert_eq!(
            AppTab::General.compact_primary_ops(&[], &[], 3),
            vec![
                OperationType::Mount,
                OperationType::Sync,
                OperationType::Bisync
            ]
        );
        assert_eq!(
            AppTab::General.compact_primary_ops(
                &["serve".into(), "mount".into(), "sync".into(), "copy".into()],
                &[],
                3
            ),
            vec![
                OperationType::Serve,
                OperationType::Mount,
                OperationType::Sync
            ]
        );
        assert_eq!(
            AppTab::Operations.compact_primary_ops(&[], &["mount".into(), "copy".into()], 3),
            vec![OperationType::Copy]
        );
        assert_eq!(
            AppTab::Mount.compact_primary_ops(&["sync".into()], &[], 3),
            vec![OperationType::Mount]
        );
        assert_eq!(
            AppTab::Serve.compact_primary_ops(&[], &[], 1),
            vec![OperationType::Serve]
        );
        assert_eq!(
            AppTab::primary_sync_ops(&[]),
            vec![
                OperationType::Sync,
                OperationType::Bisync,
                OperationType::Copy
            ]
        );
        assert_eq!(
            AppTab::more_sync_ops(&AppTab::primary_sync_ops(&[])),
            vec![
                OperationType::Move,
                OperationType::Check,
                OperationType::Delete,
                OperationType::Copyurl,
                OperationType::Archivecreate,
                OperationType::Cryptcheck
            ]
        );
        assert_eq!(
            AppTab::primary_sync_ops(&["mount".into(), "copy".into()]),
            vec![OperationType::Copy]
        );
        assert_eq!(
            AppTab::primary_sync_ops(&[
                "check".into(),
                "delete".into(),
                "copyurl".into(),
                "sync".into()
            ]),
            vec![
                OperationType::Check,
                OperationType::Delete,
                OperationType::Copyurl
            ]
        );
        let more = AppTab::more_sync_ops(&AppTab::primary_sync_ops(&[
            "check".into(),
            "delete".into(),
            "copyurl".into(),
            "sync".into(),
        ]));
        assert!(more.contains(&OperationType::Sync));
        assert!(!more.contains(&OperationType::Check));
        assert_eq!(more.len(), 6);
        assert_eq!(
            AppTab::primary_sync_ops(&["mount".into()]),
            vec![
                OperationType::Sync,
                OperationType::Bisync,
                OperationType::Copy
            ]
        );
        assert_eq!(OperationType::SYNC_TYPES.len(), 9);
    }

    #[test]
    fn merges_live_types_with_fallback() {
        let live = vec!["http".into(), "s3".into(), "webdav".into()];
        let merged = serve_types_or_default(&live);
        assert_eq!(merged[0], "http");
        assert!(merged.contains(&"ftp".to_string()));
        assert_eq!(merged.iter().filter(|s| *s == "s3").count(), 1);
        assert_eq!(
            mount_types_or_default(&["cmount".into()]),
            vec!["cmount", "mount"]
        );
        assert_eq!(selected_or(&merged, 0, "http"), "http");
        assert_eq!(selected_or(&merged, 99, "http"), "http");
        assert_eq!(combo_names(&merged).len(), merged.len());
        assert_eq!(
            serve_types_or_default(&[]).len(),
            OperationType::SERVE_TYPES.len()
        );
    }
}
