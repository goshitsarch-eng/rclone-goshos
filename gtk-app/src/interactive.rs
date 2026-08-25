//! Interactive rclone config flow — mirrors Angular `remote-config.utils`
//! and `RemoteCreationOrchestrator`.

use crate::providers::{parse_config_step, ConfigStep};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum InteractiveAnswer {
    Text(String),
    Bool(bool),
    Number(i64),
    Empty,
}

impl InteractiveAnswer {
    pub fn from_json(value: &Value) -> Self {
        match value {
            Value::Bool(b) => Self::Bool(*b),
            Value::Number(n) => n
                .as_i64()
                .map(Self::Number)
                .unwrap_or_else(|| Self::Text(n.to_string())),
            Value::String(s) if s.is_empty() => Self::Empty,
            Value::String(s) => Self::Text(s.clone()),
            Value::Null => Self::Empty,
            other => Self::Text(other.to_string()),
        }
    }

    pub fn as_rc_result(&self, option_type: &str) -> Value {
        if option_type == "bool" {
            Value::String(convert_bool_answer_to_string(self))
        } else {
            match self {
                Self::Text(s) => Value::String(s.clone()),
                Self::Bool(b) => Value::String(if *b { "true" } else { "false" }.into()),
                Self::Number(n) => Value::Number((*n).into()),
                Self::Empty => Value::String(String::new()),
            }
        }
    }

    pub fn as_string(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Bool(b) => if *b { "true" } else { "false" }.into(),
            Self::Number(n) => n.to_string(),
            Self::Empty => String::new(),
        }
    }

    pub fn is_blank(&self) -> bool {
        match self {
            Self::Empty => true,
            Self::Text(s) => s.trim().is_empty(),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InteractiveFlowState {
    pub is_active: bool,
    pub question: Option<ConfigStep>,
    pub answer: InteractiveAnswer,
    pub is_processing: bool,
}

impl Default for InteractiveFlowState {
    fn default() -> Self {
        create_initial_interactive_flow_state()
    }
}

pub fn create_initial_interactive_flow_state() -> InteractiveFlowState {
    InteractiveFlowState {
        is_active: false,
        question: None,
        answer: InteractiveAnswer::Empty,
        is_processing: false,
    }
}

pub fn convert_bool_answer_to_string(answer: &InteractiveAnswer) -> String {
    match answer {
        InteractiveAnswer::Bool(true) => "true".into(),
        InteractiveAnswer::Text(s) if s.eq_ignore_ascii_case("true") => "true".into(),
        _ => "false".into(),
    }
}

pub fn update_interactive_answer(
    mut state: InteractiveFlowState,
    answer: InteractiveAnswer,
) -> InteractiveFlowState {
    state.answer = answer;
    state
}

pub fn get_default_answer_from_step(step: &ConfigStep) -> InteractiveAnswer {
    let Some(opt) = &step.option else {
        return InteractiveAnswer::Empty;
    };
    if opt.type_name == "bool" {
        if let Value::Bool(b) = &opt.value {
            return InteractiveAnswer::Bool(*b);
        }
        if !opt.value_str.is_empty() {
            return InteractiveAnswer::Bool(opt.value_str.eq_ignore_ascii_case("true"));
        }
        if !opt.default_str.is_empty() {
            return InteractiveAnswer::Bool(opt.default_str.eq_ignore_ascii_case("true"));
        }
        return match &opt.default {
            Value::Bool(b) => InteractiveAnswer::Bool(*b),
            _ => InteractiveAnswer::Bool(true),
        };
    }

    let mut def_val = if !opt.value_str.is_empty() {
        opt.value_str.clone()
    } else if !opt.default_str.is_empty() {
        opt.default_str.clone()
    } else if !opt.default.is_null() {
        match &opt.default {
            Value::String(s) => s.clone(),
            other => other.to_string().trim_matches('"').to_string(),
        }
    } else if let Some((example, _)) = opt.examples.first() {
        example.clone()
    } else {
        String::new()
    };

    if !opt.examples.is_empty() {
        let has_exact = opt.examples.iter().any(|(v, _)| v == &def_val);
        if !has_exact {
            if let Ok(num) = def_val.parse::<usize>() {
                if num >= 1 && num <= opt.examples.len() {
                    def_val = opt.examples[num - 1].0.clone();
                }
            }
        }
    }

    if def_val.is_empty() {
        InteractiveAnswer::Empty
    } else {
        InteractiveAnswer::Text(def_val)
    }
}

/// Apply a `config/create` or `config/update` interactive response.
/// Empty `State` means rclone finished (same as Angular orchestrator).
pub fn apply_interactive_response(value: &Value) -> InteractiveFlowState {
    let step = parse_config_step(value);
    if step.state.is_empty() {
        return create_initial_interactive_flow_state();
    }
    let answer = get_default_answer_from_step(&step);
    InteractiveFlowState {
        is_active: true,
        question: Some(step),
        answer,
        is_processing: false,
    }
}

/// Angular `allowsCustomValue`: examples exist and are not exclusive.
pub fn allows_custom_value(option: &crate::providers::ProviderOption) -> bool {
    !option.examples.is_empty() && !option.exclusive
}

pub fn example_label(value: &str, help: &str) -> String {
    if help.is_empty() || help == value {
        value.to_string()
    } else {
        format!("{help} ({value})")
    }
}

pub fn selected_example_index(
    option: &crate::providers::ProviderOption,
    answer: &str,
) -> Option<usize> {
    option
        .examples
        .iter()
        .position(|(value, _)| value == answer)
}

pub fn default_value_text(option: &crate::providers::ProviderOption) -> Option<String> {
    if option.default_str.is_empty() {
        None
    } else {
        Some(option.default_str.clone())
    }
}

pub fn is_continue_disabled(state: &InteractiveFlowState) -> bool {
    if state.is_processing {
        return true;
    }
    let Some(question) = &state.question else {
        return true;
    };
    if question.error.is_some() {
        return true;
    }
    let Some(opt) = &question.option else {
        return false;
    };
    opt.required && state.answer.is_blank()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bool_answers_serialize_as_strings() {
        assert_eq!(
            convert_bool_answer_to_string(&InteractiveAnswer::Bool(true)),
            "true"
        );
        assert_eq!(
            convert_bool_answer_to_string(&InteractiveAnswer::Text("TRUE".into())),
            "true"
        );
        assert_eq!(
            convert_bool_answer_to_string(&InteractiveAnswer::Text("no".into())),
            "false"
        );
        assert_eq!(
            InteractiveAnswer::Bool(true).as_rc_result("bool"),
            json!("true")
        );
    }

    #[test]
    fn default_answer_uses_examples_and_1_based_index() {
        let step = parse_config_step(&json!({
            "State": "s",
            "Option": {
                "Name": "scope",
                "Type": "string",
                "DefaultStr": "2",
                "Examples": [
                    {"Value": "drive.readonly", "Help": "ro"},
                    {"Value": "drive", "Help": "full"}
                ]
            }
        }));
        assert_eq!(
            get_default_answer_from_step(&step),
            InteractiveAnswer::Text("drive".into())
        );
    }

    #[test]
    fn empty_state_completes_flow() {
        let state = apply_interactive_response(&json!({ "State": "" }));
        assert!(!state.is_active);
        assert!(state.question.is_none());
    }

    #[test]
    fn active_question_keeps_flow_open() {
        let state = apply_interactive_response(&json!({
            "State": "abc",
            "Option": {"Name": "client_id", "Help": "id", "Required": true, "Type": "string"}
        }));
        assert!(state.is_active);
        assert!(is_continue_disabled(&state));
        let state = update_interactive_answer(state, InteractiveAnswer::Text("x".into()));
        assert!(!is_continue_disabled(&state));
    }

    #[test]
    fn bool_default_is_true_when_unspecified() {
        let step = parse_config_step(&json!({
            "State": "s",
            "Option": {"Name": "ok", "Type": "bool"}
        }));
        assert_eq!(
            get_default_answer_from_step(&step),
            InteractiveAnswer::Bool(true)
        );
    }

    #[test]
    fn custom_value_and_example_helpers() {
        let exclusive = parse_config_step(&json!({
            "State": "s",
            "Option": {
                "Name": "scope",
                "Type": "string",
                "DefaultStr": "drive",
                "Exclusive": true,
                "Examples": [
                    {"Value": "drive.readonly", "Help": "Read only"},
                    {"Value": "drive", "Help": "Full"}
                ]
            }
        }));
        let option = exclusive.option.as_ref().unwrap();
        assert!(!allows_custom_value(option));
        assert_eq!(selected_example_index(option, "drive"), Some(1));
        assert_eq!(default_value_text(option).as_deref(), Some("drive"));
        assert_eq!(example_label("drive", "Full"), "Full (drive)");
        assert_eq!(example_label("drive", ""), "drive");

        let open = parse_config_step(&json!({
            "State": "s",
            "Option": {
                "Name": "region",
                "Type": "string",
                "Examples": [{"Value": "us", "Help": "US"}]
            }
        }));
        assert!(allows_custom_value(open.option.as_ref().unwrap()));
    }
}
