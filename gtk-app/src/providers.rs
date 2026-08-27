//! Parse rclone `config/providers` into form fields for the remote wizard.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderOption {
    pub name: String,
    pub help: String,
    pub required: bool,
    pub advanced: bool,
    pub is_password: bool,
    pub exclusive: bool,
    pub type_name: String,
    pub default: serde_json::Value,
    pub default_str: String,
    pub value: serde_json::Value,
    pub value_str: String,
    pub examples: Vec<(String, String)>,
    pub provider: String,
    pub example_providers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    pub name: String,
    pub description: String,
    pub prefix: String,
    pub options: Vec<ProviderOption>,
}

impl Provider {
    pub fn required_options(&self) -> impl Iterator<Item = &ProviderOption> {
        self.options.iter().filter(|o| o.required && !o.advanced)
    }

    pub fn basic_options(&self) -> impl Iterator<Item = &ProviderOption> {
        self.options.iter().filter(|o| !o.advanced)
    }

    pub fn advanced_options(&self) -> impl Iterator<Item = &ProviderOption> {
        self.options.iter().filter(|o| o.advanced)
    }

    /// Angular `get_oauth_supported_remotes`: a `token` option whose help
    /// describes an OAuth access-token JSON blob.
    pub fn supports_oauth(&self) -> bool {
        self.options.iter().any(|option| {
            option.name == "token" && option.help.contains("OAuth Access Token as a JSON blob")
        })
    }
}

pub fn oauth_supported_providers(providers: &[Provider]) -> Vec<Provider> {
    providers
        .iter()
        .filter(|provider| provider.supports_oauth())
        .cloned()
        .collect()
}

pub fn parse_providers(value: &Value) -> Vec<Provider> {
    let array = value
        .get("providers")
        .and_then(|v| v.as_array())
        .or_else(|| value.as_array())
        .cloned()
        .unwrap_or_default();
    let mut providers: Vec<Provider> = array.iter().filter_map(parse_provider).collect();
    providers.sort_by_key(|p| provider_sort_key(&p.name));
    providers
}

/// Pin everyday backends first so local/sftp are not buried under Amazon Cloud Drive.
pub fn provider_sort_key(name: &str) -> (usize, String) {
    const PINNED: &[&str] = &[
        "local", "alias", "drive", "s3", "dropbox", "onedrive", "sftp", "webdav", "ftp", "crypt",
    ];
    let pin = PINNED
        .iter()
        .position(|n| *n == name)
        .unwrap_or(PINNED.len());
    (pin, name.to_lowercase())
}

fn parse_provider(value: &Value) -> Option<Provider> {
    let name = value
        .get("Name")
        .or_else(|| value.get("name"))
        .and_then(|x| x.as_str())?
        .to_string();
    let prefix = value
        .get("Prefix")
        .or_else(|| value.get("prefix"))
        .and_then(|x| x.as_str())
        .unwrap_or(&name)
        .to_string();
    let description = value
        .get("Description")
        .or_else(|| value.get("description"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let options = value
        .get("Options")
        .or_else(|| value.get("options"))
        .and_then(|x| x.as_array())
        .map(|arr| arr.iter().filter_map(parse_option).collect())
        .unwrap_or_default();
    Some(Provider {
        name,
        description,
        prefix,
        options,
    })
}

fn parse_option(value: &Value) -> Option<ProviderOption> {
    let name = value
        .get("Name")
        .or_else(|| value.get("name"))?
        .as_str()?
        .to_string();
    if name.is_empty() {
        return None;
    }
    let mut example_providers = Vec::new();
    let examples = value
        .get("Examples")
        .or_else(|| value.get("examples"))
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|ex| {
                    let val = ex
                        .get("Value")
                        .or_else(|| ex.get("value"))
                        .and_then(|x| x.as_str())?
                        .to_string();
                    let help = ex
                        .get("Help")
                        .or_else(|| ex.get("help"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    let provider = ex
                        .get("Provider")
                        .or_else(|| ex.get("provider"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    example_providers.push(provider);
                    Some((val, help))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(ProviderOption {
        help: value
            .get("Help")
            .or_else(|| value.get("help"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        required: value
            .get("Required")
            .or_else(|| value.get("required"))
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
        advanced: value
            .get("Advanced")
            .or_else(|| value.get("advanced"))
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
        is_password: value
            .get("IsPassword")
            .or_else(|| value.get("isPassword"))
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
        exclusive: value
            .get("Exclusive")
            .or_else(|| value.get("exclusive"))
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
        type_name: value
            .get("Type")
            .or_else(|| value.get("type"))
            .and_then(|x| x.as_str())
            .unwrap_or("string")
            .to_string(),
        default: value
            .get("Default")
            .or_else(|| value.get("default"))
            .cloned()
            .unwrap_or(Value::Null),
        default_str: value
            .get("DefaultStr")
            .or_else(|| value.get("defaultStr"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        value: value
            .get("Value")
            .or_else(|| value.get("value"))
            .cloned()
            .unwrap_or(Value::Null),
        value_str: value
            .get("ValueStr")
            .or_else(|| value.get("valueStr"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        examples,
        provider: value
            .get("Provider")
            .or_else(|| value.get("provider"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        example_providers,
        name,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigStep {
    pub state: String,
    pub option: Option<ProviderOption>,
    pub error: Option<String>,
    pub done: bool,
}

/// Adwaita symbolic icon for an rclone provider type (dashboard + Files sidebar).
pub fn provider_icon(provider: &str) -> &'static str {
    match provider.trim().to_ascii_lowercase().as_str() {
        "local" | "memory" => "drive-harddisk-symbolic",
        "s3" | "aws" | "b2" | "wasabi" | "minio" | "storj" | "swift" | "azureblob"
        | "azurefiles" | "internetarchive" => "network-server-symbolic",
        "ftp" | "sftp" | "http" | "webdav" | "hdfs" => "network-server-symbolic",
        "crypt" => "security-high-symbolic",
        "alias" | "combine" | "union" => "emblem-symbolic-link",
        "cache" | "chunker" | "compress" | "hasher" => "package-x-generic-symbolic",
        "" => "folder-remote-symbolic",
        _ => "folder-remote-symbolic",
    }
}

pub fn dump_remote_params(dump: &Value, remote: &str) -> Option<Value> {
    dump.get(remote).cloned().filter(|v| v.is_object())
}

pub fn dump_provider_type(params: &Value) -> Option<String> {
    params
        .get("type")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Parameters for `config/create` / `config/update` interactive re-auth.
/// Drops `name` / `type` so they are only passed as dedicated RC fields.
pub fn interactive_remote_params(dump: &Value, remote: &str) -> Option<(String, Value)> {
    let mut params = dump_remote_params(dump, remote)?;
    let r#type = dump_provider_type(&params)?;
    if let Some(obj) = params.as_object_mut() {
        obj.remove("type");
        obj.remove("name");
    }
    Some((r#type, params))
}

pub fn provider_index_by_name(providers: &[Provider], type_name: &str) -> Option<usize> {
    let wanted = type_name.trim();
    if wanted.is_empty() {
        return None;
    }
    providers
        .iter()
        .position(|p| p.name.eq_ignore_ascii_case(wanted) || p.prefix.eq_ignore_ascii_case(wanted))
}

/// Password-typed provider options for the Angular obscure apply-to-field list.
pub fn sensitive_field_labels(providers: &[Provider], type_name: &str) -> Vec<(String, String)> {
    let Some(idx) = provider_index_by_name(providers, type_name) else {
        return Vec::new();
    };
    providers[idx]
        .options
        .iter()
        .filter(|option| option.is_password)
        .map(|option| (option.name.clone(), option.name.clone()))
        .collect()
}

pub fn dump_field_text(params: &Value, name: &str) -> Option<String> {
    match params.get(name)? {
        Value::Null => None,
        Value::String(s) if s.is_empty() => None,
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        other => Some(other.to_string()),
    }
}

pub fn parse_parameters_json(text: &str) -> Result<serde_json::Map<String, Value>, String> {
    crate::flags::parse_json_object(text)
}

pub fn apply_dump_to_options(options: &mut [ProviderOption], params: &Value) {
    for option in options {
        if let Some(text) = dump_field_text(params, &option.name) {
            option.value_str = text.clone();
            option.value = params
                .get(&option.name)
                .cloned()
                .unwrap_or(Value::String(text));
        }
    }
}

pub fn parse_config_step(value: &Value) -> ConfigStep {
    let state = value
        .get("State")
        .or_else(|| value.get("state"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let error = value
        .get("Error")
        .or_else(|| value.get("error"))
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let option = value
        .get("Option")
        .or_else(|| value.get("option"))
        .and_then(parse_option);
    let done = state.is_empty();
    ConfigStep {
        state,
        option,
        error,
        done,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_provider_options() {
        let value = json!({
            "providers": [{
                "Name": "drive",
                "Description": "Google Drive",
                "Prefix": "drive",
                "Options": [
                    {"Name": "client_id", "Help": "OAuth client", "Required": false, "Advanced": false, "Type": "string"},
                    {"Name": "token", "Help": "token", "Required": true, "IsPassword": true, "Advanced": true, "Type": "string"},
                    {"Name": "scope", "Help": "scope", "Examples": [{"Value": "drive", "Help": "full"}]}
                ]
            }]
        });
        let providers = parse_providers(&value);
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].name, "drive");
        assert_eq!(providers[0].options.len(), 3);
        assert_eq!(providers[0].required_options().count(), 0);
        assert_eq!(providers[0].advanced_options().count(), 1);
        assert!(providers[0].options[1].is_password);
        assert_eq!(providers[0].options[2].examples[0].0, "drive");
        let fields = sensitive_field_labels(&providers, "drive");
        assert_eq!(fields, vec![("token".into(), "token".into())]);
        assert!(sensitive_field_labels(&providers, "sftp").is_empty());
    }

    #[test]
    fn pins_everyday_providers_before_alphabetical() {
        assert!(provider_sort_key("local") < provider_sort_key("amazon cloud drive"));
        assert!(provider_sort_key("sftp") < provider_sort_key("b2"));
        assert!(provider_sort_key("drive") < provider_sort_key("dropbox"));
        let value = json!({
            "providers": [
                {"Name": "b2", "Description": "B2", "Prefix": "b2", "Options": []},
                {"Name": "local", "Description": "Local", "Prefix": "local", "Options": []},
                {"Name": "amazon cloud drive", "Description": "ACD", "Prefix": "acd", "Options": []}
            ]
        });
        let names: Vec<_> = parse_providers(&value)
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, vec!["local", "amazon cloud drive", "b2"]);
    }

    #[test]
    fn parses_interactive_config_step() {
        let step = parse_config_step(&json!({
            "State": "abc",
            "Option": {"Name": "client_id", "Help": "id", "Required": true},
            "Error": ""
        }));
        assert!(!step.done);
        assert_eq!(step.state, "abc");
        assert_eq!(step.option.unwrap().name, "client_id");
        let done = parse_config_step(&json!({}));
        assert!(done.done);
    }

    #[test]
    fn provider_icon_maps_known_types() {
        assert_eq!(provider_icon("local"), "drive-harddisk-symbolic");
        assert_eq!(provider_icon("S3"), "network-server-symbolic");
        assert_eq!(provider_icon("crypt"), "security-high-symbolic");
        assert_eq!(provider_icon("drive"), "folder-remote-symbolic");
        assert_eq!(provider_icon(""), "folder-remote-symbolic");
    }

    #[test]
    fn dump_helpers_select_provider_and_fill_options() {
        let dump = json!({
            "photos": {
                "type": "drive",
                "client_id": "abc",
                "team_drive": true,
                "chunk_size": 8
            }
        });
        let params = dump_remote_params(&dump, "photos").unwrap();
        assert_eq!(dump_provider_type(&params).as_deref(), Some("drive"));
        assert!(dump_remote_params(&dump, "missing").is_none());
        let mut providers = parse_providers(&json!({
            "providers": [
                {"Name": "s3", "Prefix": "s3", "Options": []},
                {"Name": "drive", "Prefix": "drive", "Options": [
                    {"Name": "client_id"},
                    {"Name": "team_drive"},
                    {"Name": "chunk_size"}
                ]}
            ]
        }));
        let drive_idx = provider_index_by_name(&providers, "DRIVE").expect("drive");
        assert_eq!(providers[drive_idx].name, "drive");
        apply_dump_to_options(&mut providers[drive_idx].options, &params);
        assert_eq!(providers[drive_idx].options[0].value_str, "abc");
        assert_eq!(providers[drive_idx].options[1].value_str, "true");
        assert_eq!(providers[drive_idx].options[2].value_str, "8");
        assert_eq!(dump_field_text(&params, "token"), None);
        let (kind, stripped) = interactive_remote_params(&dump, "photos").unwrap();
        assert_eq!(kind, "drive");
        assert!(stripped.get("type").is_none());
        assert_eq!(stripped["client_id"], "abc");
        assert!(interactive_remote_params(&dump, "missing").is_none());
    }

    #[test]
    fn parse_parameters_json_accepts_objects() {
        let map = parse_parameters_json(r#"{ "client_id": "abc", "team_drive": true }"#).unwrap();
        assert_eq!(map["client_id"], "abc");
        assert_eq!(map["team_drive"], true);
        assert!(parse_parameters_json("").unwrap().is_empty());
        assert!(parse_parameters_json("   ").unwrap().is_empty());
        assert!(parse_parameters_json("[1,2]").is_err());
        assert!(parse_parameters_json("{").is_err());
    }

    #[test]
    fn filters_oauth_supported_providers() {
        let providers = parse_providers(&json!({
            "providers": [
                {"Name": "local", "Prefix": "local", "Options": [
                    {"Name": "nounc", "Help": "Disable UNC"}
                ]},
                {"Name": "drive", "Prefix": "drive", "Options": [
                    {"Name": "token", "Help": "OAuth Access Token as a JSON blob."}
                ]},
                {"Name": "alias", "Prefix": "alias", "Options": [
                    {"Name": "token", "Help": "Some other token"}
                ]}
            ]
        }));
        assert!(!providers[0].supports_oauth());
        let oauth = oauth_supported_providers(&providers);
        assert_eq!(oauth.len(), 1);
        assert_eq!(oauth[0].name, "drive");
    }
}
