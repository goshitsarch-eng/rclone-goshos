//! Angular `PREDEFINED_OPTIONS` / `command-options.util` for `config/create` `opt`.

use serde_json::{json, Map, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOption {
    pub key: &'static str,
    pub label_key: &'static str,
    pub description_key: &'static str,
    pub value: bool,
}

pub const PREDEFINED_OPTIONS: &[CommandOption] = &[
    CommandOption {
        key: "obscure",
        label_key: "wizards.remoteConfig.predefinedOptions.obscure.label",
        description_key: "wizards.remoteConfig.predefinedOptions.obscure.description",
        value: true,
    },
    CommandOption {
        key: "noObscure",
        label_key: "wizards.remoteConfig.predefinedOptions.noObscure.label",
        description_key: "wizards.remoteConfig.predefinedOptions.noObscure.description",
        value: true,
    },
    CommandOption {
        key: "nonInteractive",
        label_key: "wizards.remoteConfig.predefinedOptions.nonInteractive.label",
        description_key: "wizards.remoteConfig.predefinedOptions.nonInteractive.description",
        value: true,
    },
    CommandOption {
        key: "all",
        label_key: "wizards.remoteConfig.predefinedOptions.all.label",
        description_key: "wizards.remoteConfig.predefinedOptions.all.description",
        value: true,
    },
    CommandOption {
        key: "noOutput",
        label_key: "wizards.remoteConfig.predefinedOptions.noOutput.label",
        description_key: "wizards.remoteConfig.predefinedOptions.noOutput.description",
        value: true,
    },
];

pub fn initial_command_options() -> Vec<CommandOption> {
    PREDEFINED_OPTIONS
        .iter()
        .filter(|option| option.key == "obscure")
        .cloned()
        .collect()
}

pub fn sync_non_interactive(options: &[CommandOption], is_interactive: bool) -> Vec<CommandOption> {
    let has_non_interactive = options.iter().any(|option| option.key == "nonInteractive");
    if is_interactive && !has_non_interactive {
        let mut next = options.to_vec();
        if let Some(option) = PREDEFINED_OPTIONS
            .iter()
            .find(|option| option.key == "nonInteractive")
        {
            next.push(option.clone());
        }
        return next;
    }
    if !is_interactive && has_non_interactive {
        return options
            .iter()
            .filter(|option| option.key != "nonInteractive")
            .cloned()
            .collect();
    }
    options.to_vec()
}

pub fn option_enabled(options: &[CommandOption], key: &str) -> bool {
    options
        .iter()
        .any(|option| option.key == key && option.value)
}

pub fn set_option(options: &mut Vec<CommandOption>, key: &str, enabled: bool) {
    if enabled {
        if options.iter().any(|option| option.key == key) {
            return;
        }
        if let Some(option) = PREDEFINED_OPTIONS.iter().find(|option| option.key == key) {
            options.push(option.clone());
        }
    } else {
        options.retain(|option| option.key != key);
    }
}

pub fn merge_create_opt(user: Option<Value>) -> Value {
    let mut opt_obj = json!({ "nonInteractive": true });
    if let Some(Value::Object(map)) = user {
        if let Some(obj) = opt_obj.as_object_mut() {
            obj.extend(map);
        }
    }
    opt_obj
}

pub fn build_opt(options: &[CommandOption]) -> Value {
    let mut map = Map::new();
    for option in options {
        map.insert(option.key.to_string(), json!(option.value));
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initializes_with_obscure_only() {
        let initial = initial_command_options();
        assert_eq!(initial.len(), 1);
        assert_eq!(initial[0].key, "obscure");
        assert!(initial[0].value);
        assert_eq!(PREDEFINED_OPTIONS.len(), 5);
    }

    #[test]
    fn syncs_non_interactive_like_angular() {
        let options = vec![PREDEFINED_OPTIONS[0].clone()];
        let added = sync_non_interactive(&options, true);
        assert!(added.iter().any(|o| o.key == "nonInteractive" && o.value));
        assert_eq!(
            added.iter().filter(|o| o.key == "nonInteractive").count(),
            1
        );
        let already = sync_non_interactive(&added, true);
        assert_eq!(
            already.iter().filter(|o| o.key == "nonInteractive").count(),
            1
        );
        let removed = sync_non_interactive(&added, false);
        assert!(!removed.iter().any(|o| o.key == "nonInteractive"));
        assert_eq!(sync_non_interactive(&options, false), options);
    }

    #[test]
    fn build_opt_and_toggles() {
        let mut options = initial_command_options();
        set_option(&mut options, "all", true);
        set_option(&mut options, "noOutput", true);
        set_option(&mut options, "obscure", true);
        let opt = build_opt(&options);
        assert_eq!(opt["obscure"], true);
        assert_eq!(opt["all"], true);
        assert_eq!(opt["noOutput"], true);
        assert!(opt.get("noObscure").is_none());
        set_option(&mut options, "all", false);
        assert!(!option_enabled(&options, "all"));
        assert!(option_enabled(&options, "obscure"));
        assert!(!option_enabled(&options, "noObscure"));
    }

    #[test]
    fn merge_create_opt_keeps_non_interactive_and_user_flags() {
        let merged = merge_create_opt(Some(json!({ "obscure": true, "all": true })));
        assert_eq!(merged["nonInteractive"], true);
        assert_eq!(merged["obscure"], true);
        assert_eq!(merged["all"], true);
        assert_eq!(merge_create_opt(None)["nonInteractive"], true);
    }
}
