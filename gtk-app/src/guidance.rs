//! Operation guidance banners matching Angular `app-operation-config`.

use crate::automation::is_local_watch_path;
use crate::operations::OperationType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerKind {
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpBanner {
    pub key: &'static str,
    pub kind: BannerKind,
}

pub fn operation_banners(
    op: OperationType,
    is_new_remote: bool,
    watch_enabled: bool,
    sources: &[String],
    dest: &str,
) -> Vec<OpBanner> {
    let mut banners = Vec::new();
    if is_new_remote {
        banners.push(OpBanner {
            key: "wizards.appOperation.completionNote",
            kind: BannerKind::Warning,
        });
    }
    let local_src = sources.iter().any(|s| is_local_watch_path(s));
    let remote_src = sources
        .iter()
        .any(|s| !s.is_empty() && !is_local_watch_path(s));
    let local_dst = is_local_watch_path(dest);
    let watch_possible = if op == OperationType::Bisync {
        local_src || local_dst
    } else {
        local_src
    };
    if watch_enabled && op.is_automatable() && !watch_possible {
        banners.push(OpBanner {
            key: if op == OperationType::Bisync {
                "wizards.appOperation.watchRequiresLocal"
            } else {
                "wizards.appOperation.watchRequiresLocalSource"
            },
            kind: BannerKind::Warning,
        });
    }
    if matches!(op, OperationType::Archivecreate | OperationType::Cryptcheck) {
        banners.push(OpBanner {
            key: "wizards.appOperation.coreCommandNote",
            kind: BannerKind::Info,
        });
    }
    if op == OperationType::Copyurl {
        banners.push(OpBanner {
            key: "wizards.appOperation.copyUrlFileSettingsWarning",
            kind: BannerKind::Info,
        });
    }
    if watch_enabled && op.is_automatable() && watch_possible {
        banners.push(OpBanner {
            key: "wizards.appOperation.watchFilterNote",
            kind: BannerKind::Info,
        });
        if matches!(
            op,
            OperationType::Sync | OperationType::Copy | OperationType::Move
        ) && local_src
            && remote_src
        {
            banners.push(OpBanner {
                key: "wizards.appOperation.watchMixedSourcesNote",
                kind: BannerKind::Info,
            });
        }
    }
    if matches!(op, OperationType::Check | OperationType::Cryptcheck) && sources.len() > 1 {
        banners.push(OpBanner {
            key: "wizards.appOperation.multiSourceOneWayInfo",
            kind: BannerKind::Info,
        });
    }
    banners
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_when_watch_has_no_local_source() {
        let banners = operation_banners(
            OperationType::Sync,
            false,
            true,
            &["drive:Photos".into()],
            "/tmp/out",
        );
        assert!(banners
            .iter()
            .any(|b| b.key == "wizards.appOperation.watchRequiresLocalSource"));
    }

    #[test]
    fn notes_copyurl_and_multi_source_check() {
        let copyurl = operation_banners(OperationType::Copyurl, false, false, &[], "");
        assert!(copyurl
            .iter()
            .any(|b| b.key == "wizards.appOperation.copyUrlFileSettingsWarning"));
        let check = operation_banners(
            OperationType::Check,
            true,
            false,
            &["a".into(), "b".into()],
            "",
        );
        assert!(check
            .iter()
            .any(|b| b.key == "wizards.appOperation.completionNote"));
        assert!(check
            .iter()
            .any(|b| b.key == "wizards.appOperation.multiSourceOneWayInfo"));
    }

    #[test]
    fn mixed_local_and_remote_watch_is_info() {
        let banners = operation_banners(
            OperationType::Copy,
            false,
            true,
            &["/home/ada/docs".into(), "drive:Inbox".into()],
            "drive:Backup",
        );
        assert!(banners
            .iter()
            .any(|b| b.key == "wizards.appOperation.watchFilterNote"));
        assert!(banners
            .iter()
            .any(|b| b.key == "wizards.appOperation.watchMixedSourcesNote"));
    }
}
