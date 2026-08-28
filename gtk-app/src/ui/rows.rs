//! Row and group constructors that render their text literally.
//!
//! `AdwPreferencesRow` sets `use-markup` on its title (and, for `AdwActionRow`,
//! its subtitle), and `AdwPreferencesGroup` parses its title and description as
//! markup too. None of this application's labels are markup: they are file
//! names, remote paths, profile names, error text and translated strings. When
//! any of those contain `&` or `<` — a file called `Tom & Jerry.mp4`, or the
//! shipped label "Alerts & Notifications" — Pango fails to parse them and the
//! widget renders **nothing at all**, so the entry silently disappears from the
//! list.
//!
//! Build rows through this module rather than calling the `adw` constructors
//! directly, and route group titles through [`escape`].

use adw::prelude::*;

/// Escape text for a widget that insists on parsing markup.
///
/// `AdwPreferencesGroup` has no `use-markup` property to turn off, so its title
/// and description have to be escaped instead.
pub fn escape(text: impl AsRef<str>) -> String {
    glib::markup_escape_text(text.as_ref()).to_string()
}

pub fn action_row() -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_use_markup(false);
    row
}

pub fn expander_row() -> adw::ExpanderRow {
    let row = adw::ExpanderRow::new();
    row.set_use_markup(false);
    row
}

pub fn switch_row() -> adw::SwitchRow {
    let row = adw::SwitchRow::new();
    row.set_use_markup(false);
    row
}

pub fn combo_row() -> adw::ComboRow {
    let row = adw::ComboRow::new();
    row.set_use_markup(false);
    row
}

pub fn entry_row() -> adw::EntryRow {
    let row = adw::EntryRow::new();
    row.set_use_markup(false);
    row
}

pub fn password_entry_row() -> adw::PasswordEntryRow {
    let row = adw::PasswordEntryRow::new();
    row.set_use_markup(false);
    row
}

pub fn spin_row_with_range(min: f64, max: f64, step: f64) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(min, max, step);
    row.set_use_markup(false);
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_neutralizes_markup_characters() {
        // Without this, "Alerts & Notifications" renders as an empty label.
        assert_eq!(
            escape("Alerts & Notifications"),
            "Alerts &amp; Notifications"
        );
        assert_eq!(escape("report <draft>.pdf"), "report &lt;draft&gt;.pdf");
        assert_eq!(escape("Tom & Jerry.mp4"), "Tom &amp; Jerry.mp4");
        assert_eq!(escape("<1s"), "&lt;1s");
        // Ordinary text is untouched.
        assert_eq!(escape("Preferences"), "Preferences");
        assert_eq!(escape(""), "");
        assert_eq!(escape("日本語 のファイル"), "日本語 のファイル");
    }
}
