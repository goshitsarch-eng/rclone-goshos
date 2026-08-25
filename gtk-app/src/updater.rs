//! App and rclone update checks (GitHub releases + rclone.org).

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub url: String,
    pub available: bool,
}

pub fn version_is_newer(latest: &str, current: &str) -> bool {
    let latest = normalize_version(latest);
    let current = normalize_version(current);
    if latest.is_empty() || current.is_empty() {
        return false;
    }
    compare_semver(&latest, &current) == std::cmp::Ordering::Greater
}

fn normalize_version(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('v')
        .trim_start_matches("rclone-")
        .split(['-', '+', ' '])
        .next()
        .unwrap_or("")
        .to_string()
}

fn compare_semver(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| {
        s.split('.')
            .take(3)
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let aa = parse(a);
    let bb = parse(b);
    for i in 0..3 {
        let av = aa.get(i).copied().unwrap_or(0);
        let bv = bb.get(i).copied().unwrap_or(0);
        match av.cmp(&bv) {
            std::cmp::Ordering::Equal => {}
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

pub fn parse_github_release(body: &Value, current: &str) -> Option<UpdateInfo> {
    let tag = body.get("tag_name")?.as_str()?.to_string();
    let url = body
        .get("html_url")
        .and_then(|x| x.as_str())
        .unwrap_or("https://github.com/Zarestia-Dev/rclone-manager/releases")
        .to_string();
    Some(UpdateInfo {
        available: version_is_newer(&tag, current),
        latest: tag,
        current: current.to_string(),
        url,
    })
}

pub fn fetch_app_update(current: &str) -> Result<UpdateInfo, String> {
    let resp =
        ureq::get("https://api.github.com/repos/Zarestia-Dev/rclone-manager/releases/latest")
            .set("User-Agent", "rclone-manager-gtk")
            .timeout(std::time::Duration::from_secs(8))
            .call()
            .map_err(|e| e.to_string())?;
    let body: Value = resp.into_json().map_err(|e| e.to_string())?;
    parse_github_release(&body, current).ok_or_else(|| "invalid GitHub response".into())
}

pub fn fetch_rclone_update(current: &str) -> Result<UpdateInfo, String> {
    let resp = ureq::get("https://downloads.rclone.org/version.txt")
        .set("User-Agent", "rclone-manager-gtk")
        .timeout(std::time::Duration::from_secs(8))
        .call()
        .map_err(|e| e.to_string())?;
    let text = resp.into_string().map_err(|e| e.to_string())?;
    let latest = text
        .split_whitespace()
        .find(|p| p.starts_with('v') || p.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .unwrap_or(text.trim())
        .to_string();
    Ok(UpdateInfo {
        available: version_is_newer(&latest, current),
        latest,
        current: current.to_string(),
        url: "https://rclone.org/downloads/".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compares_versions() {
        assert!(version_is_newer("v0.4.0", "0.3.2"));
        assert!(!version_is_newer("0.3.2", "0.3.2"));
        assert!(!version_is_newer("0.3.1", "0.3.2"));
        assert!(version_is_newer("1.68.0", "rclone-v1.67.3"));
    }

    #[test]
    fn parses_github_payload() {
        let info = parse_github_release(
            &json!({"tag_name": "v0.9.0", "html_url": "https://example.com/r"}),
            "0.3.2",
        )
        .unwrap();
        assert!(info.available);
        assert_eq!(info.latest, "v0.9.0");
        assert_eq!(info.url, "https://example.com/r");
    }
}
