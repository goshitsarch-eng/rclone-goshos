//! Connectivity checks for Preferences and the title-bar banner.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkCheck {
    pub url: String,
    pub ok: bool,
    pub detail: String,
}

pub fn check_links(urls: &[String], timeout_secs: u64) -> Vec<LinkCheck> {
    urls.iter()
        .map(|url| check_link(url, timeout_secs))
        .collect()
}

pub fn check_link(url: &str, timeout_secs: u64) -> LinkCheck {
    if url.trim().is_empty() {
        return LinkCheck {
            url: url.to_string(),
            ok: false,
            detail: "empty URL".into(),
        };
    }
    let timeout = Duration::from_secs(timeout_secs.max(1));
    match ureq::get(url).timeout(timeout).call() {
        Ok(resp) => LinkCheck {
            url: url.to_string(),
            ok: resp.status() < 500,
            detail: format!("HTTP {}", resp.status()),
        },
        Err(ureq::Error::Status(code, _)) => LinkCheck {
            url: url.to_string(),
            ok: code < 500,
            detail: format!("HTTP {code}"),
        },
        Err(e) => LinkCheck {
            url: url.to_string(),
            ok: false,
            detail: e.to_string(),
        },
    }
}

pub fn summarize(results: &[LinkCheck]) -> String {
    let ok = results.iter().filter(|r| r.ok).count();
    format!("{ok}/{} reachable", results.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_url_fails() {
        let result = check_link("", 1);
        assert!(!result.ok);
        assert_eq!(result.detail, "empty URL");
    }

    #[test]
    fn summarize_counts() {
        let results = vec![
            LinkCheck {
                url: "a".into(),
                ok: true,
                detail: "HTTP 200".into(),
            },
            LinkCheck {
                url: "b".into(),
                ok: false,
                detail: "timeout".into(),
            },
        ];
        assert_eq!(summarize(&results), "1/2 reachable");
    }

    #[test]
    fn invalid_scheme_fails() {
        let result = check_link("not-a-url", 1);
        assert!(!result.ok);
    }
}
