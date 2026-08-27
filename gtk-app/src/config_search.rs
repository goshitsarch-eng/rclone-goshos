//! Provider / field search — Angular `filteredRemotes` and `matchesConfigSearch`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHit {
    pub index: usize,
    pub name: String,
    pub description: String,
}

pub fn strip_cli_prefix(query: &str) -> String {
    query
        .trim()
        .trim_start_matches('-')
        .trim()
        .to_ascii_lowercase()
}

pub fn normalize_rclone_key(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c == '-' || c == ' ' { '_' } else { c })
        .collect()
}

/// Angular `matchesConfigSearch`: Name / FieldName / Help plus flex `_`/`-` matching.
pub fn matches_config_search(name: &str, help: &str, field_name: &str, query: &str) -> bool {
    if query.trim().is_empty() {
        return true;
    }
    let q = strip_cli_prefix(query);
    if q.is_empty() {
        return true;
    }
    let flex_q = normalize_rclone_key(&q);
    name.to_ascii_lowercase().contains(&q)
        || field_name.to_ascii_lowercase().contains(&q)
        || help.to_ascii_lowercase().contains(&q)
        || normalize_rclone_key(name).contains(&flex_q)
        || normalize_rclone_key(field_name).contains(&flex_q)
}

/// Angular `filteredRemotes`: match provider name or description.
pub fn filter_providers(
    query: &str,
    names: &[String],
    descriptions: &[String],
) -> Vec<ProviderHit> {
    let term = query.trim().to_ascii_lowercase();
    names
        .iter()
        .enumerate()
        .filter(|(idx, name)| {
            if term.is_empty() {
                return true;
            }
            let desc = descriptions
                .get(*idx)
                .map(String::as_str)
                .unwrap_or_default();
            name.to_ascii_lowercase().contains(&term) || desc.to_ascii_lowercase().contains(&term)
        })
        .map(|(index, name)| ProviderHit {
            index,
            name: name.clone(),
            description: descriptions.get(index).cloned().unwrap_or_default(),
        })
        .collect()
}

/// Angular `filteredProvidersView`: match example Value or Help.
pub fn filter_example_choices(query: &str, examples: &[(String, String)]) -> Vec<usize> {
    let term = query.trim().to_ascii_lowercase();
    examples
        .iter()
        .enumerate()
        .filter(|(_, (value, help))| {
            term.is_empty()
                || value.to_ascii_lowercase().contains(&term)
                || help.to_ascii_lowercase().contains(&term)
        })
        .map(|(idx, _)| idx)
        .collect()
}

/// Angular uses a searchable provider-variant list for `provider` / `vendor`
/// and for exclusive example lists that are too long for a ComboRow.
pub fn should_search_examples(field_name: &str, example_count: usize) -> bool {
    example_count > 0
        && (field_name.eq_ignore_ascii_case("provider")
            || field_name.eq_ignore_ascii_case("vendor")
            || example_count >= 12)
}

pub fn example_choice_label(value: &str, help: &str) -> String {
    if help.is_empty() {
        value.to_string()
    } else {
        format!("{value} — {help}")
    }
}

pub fn resolve_example_value(text: &str, examples: &[(String, String)]) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Some((value, _)) = examples
        .iter()
        .find(|(value, _)| value == trimmed || value.eq_ignore_ascii_case(trimmed))
    {
        return value.clone();
    }
    if let Some((value, _)) = examples.iter().find(|(value, help)| {
        example_choice_label(value, help) == trimmed
            || example_choice_label(value, help).eq_ignore_ascii_case(trimmed)
    }) {
        return value.clone();
    }
    trimmed.to_string()
}

/// Keys in a JSON object that match Angular `matchesConfigSearch`.
pub fn filter_json_keys(value: &serde_json::Value, query: &str) -> Vec<String> {
    let Some(obj) = value.as_object() else {
        return Vec::new();
    };
    obj.keys()
        .filter(|key| matches_config_search(key, "", key, query))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_name_help_and_flex_keys() {
        assert!(matches_config_search("client_id", "OAuth client", "", ""));
        assert!(matches_config_search(
            "client_id",
            "OAuth client",
            "client-id",
            "--client-id"
        ));
        assert!(matches_config_search(
            "client_id",
            "OAuth client",
            "",
            "oauth"
        ));
        assert!(matches_config_search(
            "chunk_size",
            "",
            "chunk-size",
            "chunk-size"
        ));
        assert!(!matches_config_search("token", "OAuth blob", "", "aws"));
    }

    #[test]
    fn filters_providers_by_name_or_description() {
        let names = vec!["drive".into(), "s3".into(), "sftp".into()];
        let desc = vec!["Google Drive".into(), "Amazon S3".into(), "SSH/SFTP".into()];
        let hits = filter_providers("s3", &names, &desc);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "s3");
        let amazon = filter_providers("amazon", &names, &desc);
        assert_eq!(amazon.len(), 1);
        assert_eq!(amazon[0].index, 1);
        assert_eq!(filter_providers("", &names, &desc).len(), 3);
    }

    #[test]
    fn filters_example_values_and_help() {
        let examples = vec![
            ("AWS".into(), "Amazon Web Services".into()),
            ("GCS".into(), "Google Cloud Storage".into()),
            ("Minio".into(), "Minio object storage".into()),
        ];
        assert_eq!(filter_example_choices("aws", &examples), vec![0]);
        assert_eq!(filter_example_choices("cloud", &examples), vec![1]);
        assert_eq!(filter_example_choices("", &examples).len(), 3);
    }

    #[test]
    fn searches_provider_vendor_and_long_lists() {
        assert!(should_search_examples("provider", 3));
        assert!(should_search_examples("vendor", 2));
        assert!(should_search_examples("acl", 12));
        assert!(!should_search_examples("acl", 3));
        assert!(!should_search_examples("provider", 0));
    }

    #[test]
    fn resolves_example_value_from_label_or_raw() {
        let examples = vec![
            ("AWS".into(), "Amazon Web Services".into()),
            ("Minio".into(), "Minio object storage".into()),
        ];
        assert_eq!(resolve_example_value("AWS", &examples), "AWS");
        assert_eq!(
            resolve_example_value("AWS — Amazon Web Services", &examples),
            "AWS"
        );
        assert_eq!(resolve_example_value("minio", &examples), "Minio");
        assert_eq!(resolve_example_value("custom", &examples), "custom");
        assert_eq!(
            example_choice_label("AWS", "Amazon Web Services"),
            "AWS — Amazon Web Services"
        );
    }

    #[test]
    fn filters_json_object_keys() {
        let value = serde_json::json!({
            "chunk_size": "5Mi",
            "acl": "private",
            "token": "secret"
        });
        assert_eq!(
            filter_json_keys(&value, "chunk"),
            vec!["chunk_size".to_string()]
        );
        assert_eq!(filter_json_keys(&value, "--acl").len(), 1);
        assert!(filter_json_keys(&value, "").len() >= 3);
        assert!(filter_json_keys(&serde_json::json!([]), "acl").is_empty());
    }
}
