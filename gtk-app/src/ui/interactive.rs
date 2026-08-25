//! Shared interactive rclone config + OAuth helper widgets
//! (Angular `InteractiveConfigStep` + `oauth-helper`).

use super::AppCtx;
use crate::interactive::{
    apply_interactive_response, is_continue_disabled, update_interactive_answer, InteractiveAnswer,
    InteractiveFlowState,
};
use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
pub struct InteractivePanel {
    pub root: gtk::Box,
    pub title: gtk::Label,
    pub help: gtk::Label,
    pub error: gtk::Label,
    pub answer_row: adw::EntryRow,
    pub answer_switch: adw::SwitchRow,
    pub example_row: adw::ComboRow,
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
        let title = gtk::Label::new(Some(&ctx.t_or(
            "wizards.remoteConfig.configRequired",
            "Interactive configuration",
        )));
        title.add_css_class("title-4");
        title.set_xalign(0.0);
        let help = gtk::Label::new(Some(&ctx.t_or(
            "wizards.remoteConfig.nextQuestionHelp",
            "Authorize the provider or answer rclone's configuration questions.",
        )));
        help.set_wrap(true);
        help.set_xalign(0.0);
        help.add_css_class("dim-label");
        let error = gtk::Label::new(None);
        error.add_css_class("error");
        error.set_wrap(true);
        error.set_xalign(0.0);
        let answer_row = adw::EntryRow::new();
        answer_row.set_title(&ctx.t_or("wizards.remoteConfig.enterValue", "Answer"));
        let answer_switch = adw::SwitchRow::new();
        answer_switch.set_title(&ctx.t_or("wizards.remoteConfig.yes", "Yes / enabled"));
        let example_row = adw::ComboRow::new();
        example_row.set_title(&ctx.t_or("wizards.remoteConfig.chooseOption", "Choose an option"));
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
        root.append(&title);
        root.append(&help);
        root.append(&error);
        root.append(&example_row);
        root.append(&answer_row);
        root.append(&answer_switch);
        root.append(&oauth.root);
        root.append(&buttons);
        root.set_visible(false);
        Self {
            root,
            title,
            help,
            error,
            answer_row,
            answer_switch,
            example_row,
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
            &self.help,
            &self.error,
            &self.answer_row,
            &self.answer_switch,
            &self.example_row,
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

pub fn apply_question_widgets(
    ctx: &AppCtx,
    flow: &InteractiveFlowState,
    title: &gtk::Label,
    help: &gtk::Label,
    error: &gtk::Label,
    answer_row: &adw::EntryRow,
    answer_switch: &adw::SwitchRow,
    example_row: &adw::ComboRow,
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
        return;
    };
    if let Some(option) = &step.option {
        title.set_text(&option.name);
        help.set_text(&option.help);
        error.set_text(step.error.as_deref().unwrap_or(""));
        answer_switch.set_visible(option.type_name == "bool");
        answer_row.set_visible(option.type_name != "bool");
        if option.type_name == "bool" {
            answer_switch.set_active(matches!(flow.answer, InteractiveAnswer::Bool(true)));
        } else {
            answer_row.set_text(&flow.answer.as_string());
            if option.is_password {
                if let Some(child) = answer_row.first_child() {
                    if let Ok(editable) = child.downcast::<gtk::Text>() {
                        editable.set_visibility(false);
                    }
                }
            }
        }
        if option.examples.is_empty() {
            example_row.set_visible(false);
        } else {
            example_row.set_visible(true);
            let labels: Vec<String> = option
                .examples
                .iter()
                .map(|(v, h)| {
                    if h.is_empty() {
                        v.clone()
                    } else {
                        format!("{v} — {h}")
                    }
                })
                .collect();
            let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
            example_row.set_model(Some(&gtk::StringList::new(&refs)));
        }
    } else {
        title.set_text(&ctx.t_or(
            "wizards.remoteConfig.authenticationMethod",
            "Continue authorization",
        ));
        help.set_text(
            step.error
                .as_deref()
                .unwrap_or("Complete the next step in the browser."),
        );
    }
}

pub fn poll_oauth_url(ctx: &AppCtx) -> Option<String> {
    ctx.client()
        .and_then(|client| client.oauth_status().ok())
        .and_then(|(_, url)| url)
}
