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
}

pub fn parse_providers(value: &Value) -> Vec<Provider> {
    let array = value
        .get("providers")
        .and_then(|v| v.as_array())
        .or_else(|| value.as_array())
        .cloned()
        .unwrap_or_default();
    let mut providers: Vec<Provider> = array.iter().filter_map(parse_provider).collect();
    providers.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    providers
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
}
