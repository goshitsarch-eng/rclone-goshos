//! Shared interactive rclone config + OAuth helper widgets
//! (Angular `InteractiveConfigStep` + `oauth-helper`).

use super::AppCtx;
use crate::interactive::{
    allows_custom_value, apply_interactive_response, default_value_text, example_label,
    is_continue_disabled, selected_example_index, update_interactive_answer, InteractiveAnswer,
    InteractiveFlowState,
};
use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
pub struct InteractivePanel {
    pub root: gtk::Box,
    pub title: gtk::Label,
    pub required: gtk::Label,
    pub help: gtk::Label,
    pub default_info: gtk::Label,
    pub error: gtk::Label,
    pub answer_row: adw::EntryRow,
    pub peek_btn: gtk::ToggleButton,
    pub answer_switch: adw::SwitchRow,
    pub example_row: adw::ComboRow,
    pub custom_hint: gtk::Label,
    pub validation: gtk::Label,
    pub oauth: OAuthHelper,
    pub continue_btn: gtk::Button,
    pub cancel_btn: gtk::Button,
    pub flow: Rc<RefCell<InteractiveFlowState>>,
}

#[derive(Clone)]
pub struct OAuthHelper {
    pub root: gtk::Box,
    pub link: gtk::LinkButton,
    pub copy: gtk::Button,
    pub status: gtk::Label,
}

impl OAuthHelper {
    pub fn new(ctx: &AppCtx) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let prompt = gtk::Label::new(Some(&ctx.t_or(
            "modals.oauth.manualOpenPrompt",
            "If the browser did not open, use this authorization link:",
        )));
        prompt.set_wrap(true);
        prompt.set_xalign(0.0);
        prompt.add_css_class("dim-label");
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let link = gtk::LinkButton::new("https://rclone.org");
        link.set_label("");
        link.set_halign(gtk::Align::Start);
        link.set_hexpand(true);
        let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy.set_tooltip_text(Some(&ctx.t_or("modals.oauth.copyLink", "Copy link")));
        copy.set_valign(gtk::Align::Center);
        {
            let link = link.clone();
            copy.connect_clicked(move |_| {
                let uri = link.uri();
                if !uri.is_empty() {
                    if let Some(display) = gtk::gdk::Display::default() {
                        display.clipboard().set_text(&uri);
                    }
                }
            });
        }
        actions.append(&link);
        actions.append(&copy);
        let status = gtk::Label::new(None);
        status.add_css_class("dim-label");
        status.set_xalign(0.0);
        status.set_wrap(true);
        root.append(&prompt);
        root.append(&actions);
        root.append(&status);
        root.set_visible(false);
        Self {
            root,
            link,
            copy,
            status,
        }
    }

    pub fn set_url(&self, ctx: &AppCtx, url: Option<&str>) {
        match url.filter(|u| !u.is_empty()) {
            Some(url) => {
                self.root.set_visible(true);
                self.link.set_uri(url);
                self.link.set_label(url);
                self.status.set_text(&ctx.t_or(
                    "modals.oauth.openLink",
                    "Open the authorization link in your browser",
                ));
            }
            None => {
                self.root.set_visible(false);
                self.status.set_text("");
            }
        }
    }

    pub fn set_status(&self, text: &str) {
        self.status.set_text(text);
        if !text.is_empty() {
            self.root.set_visible(true);
        }
    }
}

impl InteractivePanel {
    pub fn new(ctx: &AppCtx) -> Self {
        let heading = gtk::Label::new(Some(&ctx.t_or(
            "wizards.remoteConfig.additionalConfig",
            "Additional configuration",
        )));
        heading.add_css_class("title-3");
        heading.set_xalign(0.0);
        let subtitle = gtk::Label::new(Some(&ctx.t_or(
            "wizards.remoteConfig.setupSubtitle",
            "Authorize the provider or answer rclone's configuration questions.",
        )));
        subtitle.set_wrap(true);
        subtitle.set_xalign(0.0);
        subtitle.add_css_class("dim-label");
        let title = gtk::Label::new(Some(&ctx.t_or(
            "wizards.remoteConfig.configRequired",
            "Interactive configuration",
        )));
        title.add_css_class("title-4");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        let required = gtk::Label::new(Some(
            &ctx.t_or("wizards.remoteConfig.requiredBadge", "Required"),
        ));
        required.add_css_class("accent");
        required.add_css_class("caption");
        required.set_visible(false);
        let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        title_row.append(&title);
        title_row.append(&required);
        let help = gtk::Label::new(Some(&ctx.t_or(
            "wizards.remoteConfig.nextQuestionHelp",
            "Authorize the provider or answer rclone's configuration questions.",
        )));
        help.set_wrap(true);
        help.set_xalign(0.0);
        help.add_css_class("dim-label");
        let default_info = gtk::Label::new(None);
        default_info.add_css_class("dim-label");
        default_info.add_css_class("caption");
        default_info.set_xalign(0.0);
        default_info.set_wrap(true);
        let error = gtk::Label::new(None);
        error.add_css_class("error");
        error.set_wrap(true);
        error.set_xalign(0.0);
        let answer_row = adw::EntryRow::new();
        answer_row.set_title(&ctx.t_or("wizards.remoteConfig.enterValue", "Answer"));
        let peek_btn = gtk::ToggleButton::new();
        peek_btn.set_icon_name("view-reveal-symbolic");
        peek_btn.set_tooltip_text(Some(&ctx.t_or(
            "wizards.remoteConfig.togglePassword",
            "Toggle password visibility",
        )));
        peek_btn.add_css_class("flat");
        peek_btn.set_visible(false);
        {
            let answer_row = answer_row.clone();
            peek_btn.connect_toggled(move |btn| {
                set_entry_visibility(&answer_row, btn.is_active());
            });
        }
        answer_row.add_suffix(&peek_btn);
        let answer_switch = adw::SwitchRow::new();
        answer_switch.set_title(&ctx.t_or("wizards.remoteConfig.yes", "Yes"));
        let example_row = adw::ComboRow::new();
        example_row.set_title(&ctx.t_or("wizards.remoteConfig.chooseOption", "Choose an option"));
        let custom_hint = gtk::Label::new(Some(&ctx.t_or(
            "wizards.remoteConfig.orEnterCustom",
            "Or enter a custom value",
        )));
        custom_hint.add_css_class("dim-label");
        custom_hint.add_css_class("caption");
        custom_hint.set_xalign(0.0);
        custom_hint.set_visible(false);
        let validation = gtk::Label::new(None);
        validation.add_css_class("dim-label");
        validation.add_css_class("caption");
        validation.set_xalign(0.0);
        validation.set_wrap(true);
        let oauth = OAuthHelper::new(ctx);
        let continue_btn = gtk::Button::with_label(&ctx.t_or(
            "wizards.remoteConfig.readyToContinue",
            "Continue / Authorize",
        ));
        continue_btn.add_css_class("suggested-action");
        let cancel_btn =
            gtk::Button::with_label(&ctx.t_or("modals.remoteConfig.cancelOauth", "Cancel OAuth"));
        cancel_btn.add_css_class("destructive-action");
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.append(&continue_btn);
        buttons.append(&cancel_btn);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.set_margin_top(12);
        root.append(&heading);
        root.append(&subtitle);
        root.append(&title_row);
        root.append(&help);
        root.append(&default_info);
        root.append(&error);
        root.append(&example_row);
        root.append(&custom_hint);
        root.append(&answer_row);
        root.append(&answer_switch);
        root.append(&validation);
        root.append(&oauth.root);
        root.append(&buttons);
        root.set_visible(false);
        Self {
            root,
            title,
            required,
            help,
            default_info,
            error,
            answer_row,
            peek_btn,
            answer_switch,
            example_row,
            custom_hint,
            validation,
            oauth,
            continue_btn,
            cancel_btn,
            flow: Rc::new(RefCell::new(InteractiveFlowState::default())),
        }
    }

    pub fn apply(&self, ctx: &AppCtx, flow: InteractiveFlowState) {
        apply_question_widgets(
            ctx,
            &flow,
            &self.title,
            &self.required,
            &self.help,
            &self.default_info,
            &self.error,
            &self.answer_row,
            &self.peek_btn,
            &self.answer_switch,
            &self.example_row,
            &self.custom_hint,
            &self.validation,
        );
        self.continue_btn
            .set_sensitive(!is_continue_disabled(&flow));
        self.root.set_visible(flow.is_active);
        *self.flow.borrow_mut() = flow;
    }

    pub fn current_answer(&self) -> InteractiveAnswer {
        current_answer(
            &self.flow.borrow(),
            &self.answer_row,
            &self.answer_switch,
            &self.example_row,
        )
    }

    pub fn apply_response(&self, ctx: &AppCtx, value: &serde_json::Value) -> InteractiveFlowState {
        let next = apply_interactive_response(value);
        self.apply(ctx, next.clone());
        next
    }
}

pub fn current_answer(
    flow: &InteractiveFlowState,
    answer_row: &adw::EntryRow,
    answer_switch: &adw::SwitchRow,
    example_row: &adw::ComboRow,
) -> InteractiveAnswer {
    let option = flow.question.as_ref().and_then(|q| q.option.as_ref());
    if let Some(option) = option {
        if option.type_name == "bool" {
            return InteractiveAnswer::Bool(answer_switch.is_active());
        }
        if !option.examples.is_empty() && option.exclusive {
            if let Some((value, _)) = option.examples.get(example_row.selected() as usize) {
                return InteractiveAnswer::Text(value.clone());
            }
        }
    }
    let text = answer_row.text().to_string();
    if text.is_empty() {
        update_interactive_answer(flow.clone(), InteractiveAnswer::Empty).answer
    } else {
        InteractiveAnswer::Text(text)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn apply_question_widgets(
    ctx: &AppCtx,
    flow: &InteractiveFlowState,
    title: &gtk::Label,
    required: &gtk::Label,
    help: &gtk::Label,
    default_info: &gtk::Label,
    error: &gtk::Label,
    answer_row: &adw::EntryRow,
    peek_btn: &gtk::ToggleButton,
    answer_switch: &adw::SwitchRow,
    example_row: &adw::ComboRow,
    custom_hint: &gtk::Label,
    validation: &gtk::Label,
) {
    let Some(step) = &flow.question else {
        title.set_text(&ctx.t_or(
            "wizards.remoteConfig.readyToContinue",
            "Authorization complete",
        ));
        help.set_text(&ctx.t_or(
            "wizards.remoteConfig.setupSubtitle",
            "rclone finished the interactive flow.",
        ));
        error.set_text("");
        required.set_visible(false);
        default_info.set_visible(false);
        peek_btn.set_visible(false);
        custom_hint.set_visible(false);
        validation.set_visible(false);
        answer_row.set_visible(false);
        answer_switch.set_visible(false);
        example_row.set_visible(false);
        return;
    };
    if let Some(option) = &step.option {
        title.set_text(&option.name);
        let help_text = if option.help.is_empty() {
            ctx.t_or(
                "wizards.remoteConfig.nextQuestionHelp",
                "Authorize the provider or answer rclone's configuration questions.",
            )
        } else {
            option.help.clone()
        };
        help.set_text(&help_text);
        error.set_text(step.error.as_deref().unwrap_or(""));
        required.set_visible(option.required);
        if let Some(default) = default_value_text(option) {
            default_info.set_text(&format!(
                "{} {}",
                ctx.t_or("wizards.remoteConfig.defaultValue", "Default:"),
                default
            ));
            default_info.set_visible(true);
        } else {
            default_info.set_visible(false);
        }
        let is_bool = option.type_name == "bool";
        answer_switch.set_visible(is_bool);
        let show_custom = !is_bool && (option.examples.is_empty() || allows_custom_value(option));
        answer_row.set_visible(show_custom);
        custom_hint.set_visible(show_custom && !option.examples.is_empty());
        peek_btn.set_visible(show_custom && option.is_password);
        if is_bool {
            let on = matches!(flow.answer, InteractiveAnswer::Bool(true));
            answer_switch.set_active(on);
            answer_switch.set_title(&if on {
                ctx.t_or("wizards.remoteConfig.yes", "Yes")
            } else {
                ctx.t_or("wizards.remoteConfig.no", "No")
            });
        } else if show_custom {
            answer_row.set_text(&flow.answer.as_string());
            if option.is_password && !peek_btn.is_active() {
                set_entry_visibility(answer_row, false);
            }
        }
        if option.examples.is_empty() {
            example_row.set_visible(false);
        } else {
            example_row.set_visible(true);
            let labels: Vec<String> = option
                .examples
                .iter()
                .map(|(v, h)| example_label(v, h))
                .collect();
            let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
            example_row.set_model(Some(&gtk::StringList::new(&refs)));
            if let Some(idx) = selected_example_index(option, &flow.answer.as_string()) {
                example_row.set_selected(idx as u32);
            }
        }
        if option.required {
            let ready = !is_continue_disabled(flow);
            validation.set_text(&if ready {
                ctx.t_or("wizards.remoteConfig.readyToContinue", "Ready to continue")
            } else {
                ctx.t_or(
                    "wizards.remoteConfig.provideValue",
                    "Please provide a value",
                )
            });
            validation.set_visible(true);
        } else {
            validation.set_visible(false);
        }
    } else {
        title.set_text(&ctx.t_or(
            "wizards.remoteConfig.authenticationMethod",
            "Continue authorization",
        ));
        help.set_text(&step.error.clone().unwrap_or_else(|| {
            ctx.t_or(
                "modals.oauth.openLink",
                "Complete the next step in the browser.",
            )
        }));
        required.set_visible(false);
        default_info.set_visible(false);
        peek_btn.set_visible(false);
        custom_hint.set_visible(false);
        validation.set_visible(false);
        answer_row.set_visible(false);
        answer_switch.set_visible(false);
        example_row.set_visible(false);
    }
}

fn set_entry_visibility(row: &adw::EntryRow, visible: bool) {
    if let Some(text) = find_text_widget(row.upcast_ref()) {
        text.set_visibility(visible);
    }
}

fn find_text_widget(widget: &gtk::Widget) -> Option<gtk::Text> {
    if let Ok(text) = widget.clone().downcast::<gtk::Text>() {
        return Some(text);
    }
    let mut child = widget.first_child();
    while let Some(node) = child {
        if let Some(text) = find_text_widget(&node) {
            return Some(text);
        }
        child = node.next_sibling();
    }
    None
}

pub fn poll_oauth_url(ctx: &AppCtx) -> Option<String> {
    ctx.client()
        .and_then(|client| client.oauth_status().ok())
        .and_then(|(_, url)| url)
}
