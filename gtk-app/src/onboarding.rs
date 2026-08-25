//! Onboarding card catalog — mirrors Angular `OnboardingComponent` card keys.

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingCard {
    Welcome,
    Features,
    InstallRclone,
    InstallPlugin,
    SelectConfig,
    PasswordRequired,
    SelectMainUi,
    Ready,
}

impl OnboardingCard {
    pub const ALL: &'static [Self] = &[
        Self::Welcome,
        Self::Features,
        Self::InstallRclone,
        Self::InstallPlugin,
        Self::SelectConfig,
        Self::PasswordRequired,
        Self::SelectMainUi,
        Self::Ready,
    ];

    pub fn tag(self) -> &'static str {
        match self {
            Self::Welcome => "welcome",
            Self::Features => "features",
            Self::InstallRclone => "install",
            Self::InstallPlugin => "mount",
            Self::SelectConfig => "config",
            Self::PasswordRequired => "password",
            Self::SelectMainUi => "view",
            Self::Ready => "ready",
        }
    }

    pub fn title_key(self) -> &'static str {
        match self {
            Self::Welcome => "onboarding.cards.welcome.title",
            Self::Features => "onboarding.cards.features.title",
            Self::InstallRclone => "onboarding.cards.installRclone.title",
            Self::InstallPlugin => "onboarding.cards.installPlugin.title",
            Self::SelectConfig => "onboarding.cards.selectConfig.title",
            Self::PasswordRequired => "onboarding.cards.passwordRequired.title",
            Self::SelectMainUi => "onboarding.cards.selectMainUi.title",
            Self::Ready => "onboarding.cards.ready.title",
        }
    }

    pub fn content_key(self) -> &'static str {
        match self {
            Self::Welcome => "onboarding.cards.welcome.content",
            Self::Features => "onboarding.cards.features.content",
            Self::InstallRclone => "onboarding.cards.installRclone.content",
            Self::InstallPlugin => "onboarding.cards.installPlugin.content",
            Self::SelectConfig => "onboarding.cards.selectConfig.content",
            Self::PasswordRequired => "onboarding.cards.passwordRequired.content",
            Self::SelectMainUi => "onboarding.cards.selectMainUi.content",
            Self::Ready => "onboarding.cards.ready.content",
        }
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|card| card.tag() == tag)
    }
}

/// Angular hides install / plugin / password cards when they are not needed.
pub fn visible_cards(
    rclone_installed: bool,
    plugin_installed: bool,
    password_required: bool,
) -> Vec<OnboardingCard> {
    OnboardingCard::ALL
        .iter()
        .copied()
        .filter(|card| match card {
            OnboardingCard::InstallRclone => !rclone_installed,
            OnboardingCard::InstallPlugin => !plugin_installed,
            OnboardingCard::PasswordRequired => password_required,
            _ => true,
        })
        .collect()
}

pub fn next_card(cards: &[OnboardingCard], current: OnboardingCard) -> Option<OnboardingCard> {
    let idx = cards.iter().position(|card| *card == current)?;
    cards.get(idx + 1).copied()
}

pub fn prev_card(cards: &[OnboardingCard], current: OnboardingCard) -> Option<OnboardingCard> {
    let idx = cards.iter().position(|card| *card == current)?;
    if idx == 0 {
        None
    } else {
        cards.get(idx - 1).copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MainUiOption {
    pub id: &'static str,
    pub icon: &'static str,
    pub badge_key: &'static str,
    pub title_key: &'static str,
    pub desc_key: &'static str,
}

pub const MAIN_UI_OPTIONS: &[MainUiOption] = &[
    MainUiOption {
        id: "main_menu",
        icon: "view-grid-symbolic",
        badge_key: "onboarding.uiOptions.main_menu.badge",
        title_key: "onboarding.uiOptions.main_menu.title",
        desc_key: "onboarding.uiOptions.main_menu.description",
    },
    MainUiOption {
        id: "nautilus",
        icon: "folder-symbolic",
        badge_key: "onboarding.uiOptions.nautilus.badge",
        title_key: "onboarding.uiOptions.nautilus.title",
        desc_key: "onboarding.uiOptions.nautilus.description",
    },
    MainUiOption {
        id: "flow",
        icon: "media-playlist-consecutive-symbolic",
        badge_key: "onboarding.uiOptions.flow.badge",
        title_key: "onboarding.uiOptions.flow.title",
        desc_key: "onboarding.uiOptions.flow.description",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallLocation {
    Default,
    Custom,
    Existing,
}

pub fn rclone_install_dest(location: InstallLocation, custom: &str) -> Option<PathBuf> {
    match location {
        InstallLocation::Default => Some(
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local/bin"),
        ),
        InstallLocation::Custom => {
            let trimmed = custom.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(PathBuf::from(trimmed))
            }
        }
        InstallLocation::Existing => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_optional_cards_when_healthy() {
        let cards = visible_cards(true, true, false);
        assert_eq!(
            cards,
            vec![
                OnboardingCard::Welcome,
                OnboardingCard::Features,
                OnboardingCard::SelectConfig,
                OnboardingCard::SelectMainUi,
                OnboardingCard::Ready,
            ]
        );
        assert_eq!(
            next_card(&cards, OnboardingCard::Features),
            Some(OnboardingCard::SelectConfig)
        );
        assert_eq!(prev_card(&cards, OnboardingCard::Welcome), None);
        assert_eq!(
            OnboardingCard::from_tag("view"),
            Some(OnboardingCard::SelectMainUi)
        );
    }

    #[test]
    fn keeps_install_and_password_when_needed() {
        let cards = visible_cards(false, false, true);
        assert_eq!(cards, OnboardingCard::ALL.to_vec());
        assert_eq!(
            next_card(&cards, OnboardingCard::InstallRclone),
            Some(OnboardingCard::InstallPlugin)
        );
    }

    #[test]
    fn main_ui_options_match_angular() {
        assert_eq!(MAIN_UI_OPTIONS.len(), 3);
        assert_eq!(MAIN_UI_OPTIONS[0].id, "main_menu");
        assert_eq!(MAIN_UI_OPTIONS[1].id, "nautilus");
        assert_eq!(MAIN_UI_OPTIONS[2].id, "flow");
        assert!(MAIN_UI_OPTIONS
            .iter()
            .all(|opt| opt.title_key.contains("uiOptions")));
    }

    #[test]
    fn rclone_dest_requires_custom_path() {
        assert!(rclone_install_dest(InstallLocation::Default, "")
            .unwrap()
            .ends_with(".local/bin"));
        assert!(rclone_install_dest(InstallLocation::Custom, "  ").is_none());
        assert_eq!(
            rclone_install_dest(InstallLocation::Custom, "/opt/rclone"),
            Some(PathBuf::from("/opt/rclone"))
        );
        assert!(rclone_install_dest(InstallLocation::Existing, "/bin/rclone").is_none());
    }
}
