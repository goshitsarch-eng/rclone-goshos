//! Shared remote-config / create-wizard step list and navigation rules.
//!
//! Mirrors Angular `RemoteConfigStateService.stepConfigs`, `isStepClickable`,
//! and `isNextDisabled`. Full create is Remote + 11 operations + filter / VFS /
//! backend / runtime (16 steps). Quick Add is Remote + one Operations page.

use crate::operations::OperationType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorStep {
    Remote,
    Op(OperationType),
    QuickOps,
    Helper(&'static str),
}

/// Helper sidebar order matches Angular `FLAG_TYPES` tail + `runtimeRemote`.
pub const HELPERS: &[(&str, &str, &str)] = &[
    ("filter", "Filter", "view-filter-symbolic"),
    ("vfs", "VFS", "drive-harddisk-symbolic"),
    ("backend", "Backend", "preferences-system-symbolic"),
    ("runtime", "Runtime", "emblem-system-symbolic"),
];

pub const QUICK_ADD_OPS: [OperationType; 6] = [
    OperationType::Mount,
    OperationType::Sync,
    OperationType::Copy,
    OperationType::Bisync,
    OperationType::Move,
    OperationType::Serve,
];

impl EditorStep {
    pub fn page_name(self) -> &'static str {
        match self {
            Self::Remote => "remote",
            Self::Op(op) => op.as_str(),
            Self::QuickOps => "operations",
            Self::Helper(kind) => kind,
        }
    }

    pub fn alias(self) -> &'static str {
        self.page_name()
    }

    pub fn i18n_key(self) -> String {
        match self {
            Self::Remote => "modals.remoteConfig.steps.remote".into(),
            Self::Op(op) => format!("modals.remoteConfig.steps.{}", op.as_str()),
            Self::QuickOps => "modals.quickAdd.operations.title".into(),
            Self::Helper("runtime") => "modals.remoteConfig.steps.runtimeRemote".into(),
            Self::Helper(kind) => format!("modals.remoteConfig.steps.{kind}"),
        }
    }

    pub fn fallback_label(self) -> &'static str {
        match self {
            Self::Remote => "Remote",
            Self::Op(op) => op.api_label(),
            Self::QuickOps => "Operation Options (Optional)",
            Self::Helper(kind) => HELPERS
                .iter()
                .find(|(k, _, _)| *k == kind)
                .map(|(_, label, _)| *label)
                .unwrap_or(kind),
        }
    }

    pub fn icon_name(self) -> &'static str {
        match self {
            Self::Remote => "network-server-symbolic",
            Self::Op(op) => op.icon_name(),
            Self::QuickOps => "folder-download-symbolic",
            Self::Helper(kind) => HELPERS
                .iter()
                .find(|(k, _, _)| *k == kind)
                .map(|(_, _, icon)| *icon)
                .unwrap_or("preferences-other-symbolic"),
        }
    }

    pub fn is_remote(self) -> bool {
        matches!(self, Self::Remote)
    }
}

pub fn editor_steps() -> Vec<EditorStep> {
    let mut steps = vec![EditorStep::Remote];
    steps.extend(OperationType::ALL.iter().copied().map(EditorStep::Op));
    steps.extend(HELPERS.iter().map(|(kind, _, _)| EditorStep::Helper(*kind)));
    steps
}

pub fn wizard_steps(oauth_only: bool) -> Vec<EditorStep> {
    if oauth_only {
        vec![EditorStep::Remote, EditorStep::QuickOps]
    } else {
        editor_steps()
    }
}

pub fn parse_open_step(raw: Option<&str>) -> EditorStep {
    let Some(raw) = raw else {
        return EditorStep::Remote;
    };
    let lower = raw.to_ascii_lowercase();
    if lower.is_empty() || lower == "remote" || lower == "remoteconfig" {
        return EditorStep::Remote;
    }
    if lower == "operations" || lower == "quickops" {
        return EditorStep::QuickOps;
    }
    if let Some(op) = OperationType::parse(&lower) {
        return EditorStep::Op(op);
    }
    if let Some((kind, _, _)) = HELPERS.iter().find(|(kind, _, _)| {
        *kind == lower
            || (*kind == "runtime" && matches!(lower.as_str(), "runtimeremote" | "runtime"))
    }) {
        return EditorStep::Helper(*kind);
    }
    EditorStep::Remote
}

pub fn is_step_clickable(
    target_idx: usize,
    current_idx: usize,
    remote_valid: bool,
    navigation_locked: bool,
) -> bool {
    if navigation_locked {
        return false;
    }
    if target_idx > current_idx && !remote_valid {
        return false;
    }
    true
}

pub fn is_next_disabled(
    current_is_remote: bool,
    remote_valid: bool,
    navigation_locked: bool,
) -> bool {
    if navigation_locked {
        return true;
    }
    current_is_remote && !remote_valid
}

pub fn next_step_index(current: usize, len: usize) -> Option<usize> {
    let next = current.saturating_add(1);
    (next < len).then_some(next)
}

pub fn prev_step_index(current: usize) -> Option<usize> {
    current.checked_sub(1)
}

/// Angular `remoteEditCategories` for remote-edit section jump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteEditSection {
    pub id: &'static str,
    pub i18n_key: &'static str,
    pub fallback: &'static str,
    pub icon: &'static str,
}

pub const REMOTE_EDIT_SECTIONS: &[RemoteEditSection] = &[
    RemoteEditSection {
        id: "section-general",
        i18n_key: "modals.remoteConfig.editMode.sections.general",
        fallback: "General",
        icon: "emblem-system-symbolic",
    },
    RemoteEditSection {
        id: "section-auth",
        i18n_key: "modals.remoteConfig.editMode.sections.auth",
        fallback: "Auth",
        icon: "dialog-password-symbolic",
    },
    RemoteEditSection {
        id: "section-advanced",
        i18n_key: "modals.remoteConfig.editMode.sections.advanced",
        fallback: "Advanced",
        icon: "preferences-other-symbolic",
    },
];

/// Angular `sharedSidebarTypes`: VFS / filter / backend / runtime, minus current.
/// VFS is only offered from mount, serve, filter, or backend.
pub fn shared_sidebar_types(current: EditorStep) -> Vec<EditorStep> {
    if matches!(current, EditorStep::Remote | EditorStep::QuickOps) {
        return Vec::new();
    }
    [
        EditorStep::Helper("vfs"),
        EditorStep::Helper("filter"),
        EditorStep::Helper("backend"),
        EditorStep::Helper("runtime"),
    ]
    .into_iter()
    .filter(|item| {
        if *item == current {
            return false;
        }
        if *item == EditorStep::Helper("vfs") {
            matches!(
                current,
                EditorStep::Op(OperationType::Mount)
                    | EditorStep::Op(OperationType::Serve)
                    | EditorStep::Helper("filter")
                    | EditorStep::Helper("backend")
            )
        } else {
            true
        }
    })
    .collect()
}

/// Angular `navigateToShared`: push the current target, then switch.
pub fn navigate_to_shared(
    stack: &mut Vec<EditorStep>,
    current: EditorStep,
    next: EditorStep,
) -> EditorStep {
    if current != next {
        stack.push(current);
    }
    next
}

/// Angular `returnFromShared`: pop the previous target.
pub fn return_from_shared(stack: &mut Vec<EditorStep>) -> Option<EditorStep> {
    stack.pop()
}

/// Shared helper rows are hidden while the return stack is non-empty.
pub fn show_shared_sidebar(stack: &[EditorStep]) -> bool {
    stack.is_empty()
}

/// Sidebar profile names for an operation or helper step (`default` when empty).
pub fn edit_profile_names(meta: &crate::store::RemoteMeta, step: EditorStep) -> Vec<String> {
    let mut names = match step {
        EditorStep::Op(op) => meta.profile_names(op),
        EditorStep::Helper(kind) => meta.helper_names(kind),
        _ => Vec::new(),
    };
    if names.is_empty() && !matches!(step, EditorStep::Remote | EditorStep::QuickOps) {
        names.push("default".into());
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_editor_has_sixteen_steps() {
        let steps = editor_steps();
        assert_eq!(steps.len(), 16);
        assert_eq!(steps[0], EditorStep::Remote);
        assert_eq!(steps[1], EditorStep::Op(OperationType::Mount));
        assert_eq!(steps[11], EditorStep::Op(OperationType::Cryptcheck));
        assert_eq!(steps[12], EditorStep::Helper("filter"));
        assert_eq!(steps[13], EditorStep::Helper("vfs"));
        assert_eq!(steps[14], EditorStep::Helper("backend"));
        assert_eq!(steps[15], EditorStep::Helper("runtime"));
    }

    #[test]
    fn wizard_quick_add_is_remote_plus_operations() {
        let steps = wizard_steps(true);
        assert_eq!(steps, vec![EditorStep::Remote, EditorStep::QuickOps]);
        assert_eq!(wizard_steps(false), editor_steps());
    }

    #[test]
    fn page_names_are_unique_and_stable() {
        let mut names: Vec<&str> = editor_steps().iter().map(|s| s.page_name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 16);
        assert_eq!(EditorStep::QuickOps.page_name(), "operations");
        assert_eq!(EditorStep::Helper("runtime").page_name(), "runtime");
    }

    #[test]
    fn parse_open_step_aliases() {
        assert_eq!(parse_open_step(None), EditorStep::Remote);
        assert_eq!(parse_open_step(Some("remoteConfig")), EditorStep::Remote);
        assert_eq!(
            parse_open_step(Some("copy")),
            EditorStep::Op(OperationType::Copy)
        );
        assert_eq!(
            parse_open_step(Some("runtimeRemote")),
            EditorStep::Helper("runtime")
        );
        assert_eq!(parse_open_step(Some("operations")), EditorStep::QuickOps);
        assert_eq!(parse_open_step(Some("unknown")), EditorStep::Remote);
    }

    #[test]
    fn navigation_rules_lock_and_require_valid_remote() {
        assert!(!is_step_clickable(3, 0, true, true));
        assert!(!is_step_clickable(3, 0, false, false));
        assert!(is_step_clickable(3, 0, true, false));
        assert!(is_step_clickable(0, 2, false, false));
        assert!(is_next_disabled(true, false, false));
        assert!(!is_next_disabled(true, true, false));
        assert!(is_next_disabled(false, true, true));
        assert_eq!(next_step_index(0, 16), Some(1));
        assert_eq!(next_step_index(15, 16), None);
        assert_eq!(prev_step_index(0), None);
        assert_eq!(prev_step_index(4), Some(3));
    }

    #[test]
    fn shared_sidebar_matches_angular_vfs_rule() {
        let copy = shared_sidebar_types(EditorStep::Op(OperationType::Copy));
        assert_eq!(
            copy,
            vec![
                EditorStep::Helper("filter"),
                EditorStep::Helper("backend"),
                EditorStep::Helper("runtime"),
            ]
        );
        let mount = shared_sidebar_types(EditorStep::Op(OperationType::Mount));
        assert!(mount.contains(&EditorStep::Helper("vfs")));
        assert!(!mount.contains(&EditorStep::Op(OperationType::Mount)));
        assert!(shared_sidebar_types(EditorStep::Remote).is_empty());
        assert!(
            !shared_sidebar_types(EditorStep::Helper("vfs")).contains(&EditorStep::Helper("vfs"))
        );
    }

    #[test]
    fn shared_helper_stack_pushes_and_pops() {
        let mut stack = Vec::new();
        let next = navigate_to_shared(
            &mut stack,
            EditorStep::Op(OperationType::Copy),
            EditorStep::Helper("filter"),
        );
        assert_eq!(next, EditorStep::Helper("filter"));
        assert_eq!(stack, vec![EditorStep::Op(OperationType::Copy)]);
        assert!(!show_shared_sidebar(&stack));
        assert_eq!(
            return_from_shared(&mut stack),
            Some(EditorStep::Op(OperationType::Copy))
        );
        assert!(stack.is_empty());
        assert!(show_shared_sidebar(&stack));
        assert_eq!(return_from_shared(&mut stack), None);
    }

    #[test]
    fn edit_profile_names_default_when_empty() {
        let meta = crate::store::RemoteMeta::default();
        assert_eq!(
            edit_profile_names(&meta, EditorStep::Op(OperationType::Copy)),
            vec!["default".to_string()]
        );
        assert!(edit_profile_names(&meta, EditorStep::Remote).is_empty());
        assert_eq!(REMOTE_EDIT_SECTIONS[0].id, "section-general");
        assert_eq!(REMOTE_EDIT_SECTIONS[2].id, "section-advanced");
    }
}
