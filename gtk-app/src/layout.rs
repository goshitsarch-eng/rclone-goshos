//! Dashboard, Flow, and per-backend remote layout — mirrors Angular `runtime.*_layout`.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const DASHBOARD_PANELS: &[&str] = &[
    "remotes",
    "jobs",
    "serves",
    "bandwidth",
    "system",
    "automations",
];

pub const QUICK_RUN_PANELS: &[&str] = &["quickRuns", "jobs", "serves", "automations", "bandwidth"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelLayout {
    #[serde(default)]
    pub order: Vec<String>,
    #[serde(default)]
    pub hidden: Vec<String>,
}

impl PanelLayout {
    pub fn from_value(value: &Value) -> Self {
        if let Some(arr) = value.as_array() {
            return Self {
                order: arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect(),
                hidden: Vec::new(),
            };
        }
        serde_json::from_value(value.clone()).unwrap_or_default()
    }

    pub fn to_value(&self) -> Value {
        json!({ "order": self.order, "hidden": self.hidden })
    }

    pub fn resolve(&self, catalog: &[&str]) -> Vec<(String, bool)> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for id in &self.order {
            if catalog.contains(&id.as_str()) && seen.insert(id.clone()) {
                out.push((id.clone(), !self.hidden.iter().any(|h| h == id)));
            }
        }
        for id in catalog {
            if seen.insert((*id).to_string()) {
                out.push(((*id).to_string(), !self.hidden.iter().any(|h| h == id)));
            }
        }
        out
    }

    pub fn ensure_order(&mut self, catalog: &[&str]) {
        if self.order.is_empty() {
            self.order = catalog.iter().map(|s| (*s).to_string()).collect();
        } else {
            for id in catalog {
                if !self.order.iter().any(|n| n == id) {
                    self.order.push((*id).to_string());
                }
            }
        }
    }

    pub fn move_panel(&mut self, id: &str, delta: isize, catalog: &[&str]) -> bool {
        self.ensure_order(catalog);
        let Some(idx) = self.order.iter().position(|n| n == id) else {
            return false;
        };
        let last = self.order.len().saturating_sub(1) as isize;
        let next = (idx as isize + delta).clamp(0, last) as usize;
        if next == idx {
            return false;
        }
        self.order.swap(idx, next);
        true
    }

    pub fn toggle_hidden(&mut self, id: &str) -> bool {
        if let Some(idx) = self.hidden.iter().position(|n| n == id) {
            self.hidden.remove(idx);
            true
        } else {
            self.hidden.push(id.to_string());
            false
        }
    }
}

pub fn panel_title(id: &str) -> &'static str {
    match id {
        "remotes" => "Remotes",
        "jobs" => "Jobs",
        "serves" => "Serves",
        "bandwidth" => "Bandwidth",
        "system" => "System",
        "automations" => "Automations",
        "quickRuns" => "Quick Runs",
        _ => "Panel",
    }
}

pub fn backend_key(active: &str) -> String {
    if active.is_empty() || active == "local" {
        "local".into()
    } else {
        active.to_string()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemotesLayout {
    #[serde(default)]
    pub order: Vec<String>,
    #[serde(default)]
    pub hidden: Vec<String>,
}

pub fn load_remote_layout(layouts: &Value, backend: &str) -> RemotesLayout {
    layouts
        .get(backend)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

pub fn store_remote_layout(layouts: &mut Value, backend: &str, layout: &RemotesLayout) {
    if !layouts.is_object() {
        *layouts = json!({});
    }
    if let Some(map) = layouts.as_object_mut() {
        map.insert(
            backend.to_string(),
            json!({ "order": layout.order, "hidden": layout.hidden }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_appends_unknown_catalog_ids() {
        let layout = PanelLayout {
            order: vec!["jobs".into(), "missing".into()],
            hidden: vec!["system".into()],
        };
        let resolved = layout.resolve(DASHBOARD_PANELS);
        assert_eq!(resolved[0], ("jobs".into(), true));
        assert!(resolved.iter().any(|(id, vis)| id == "remotes" && *vis));
        assert!(resolved.iter().any(|(id, vis)| id == "system" && !*vis));
        assert!(!resolved.iter().any(|(id, _)| id == "missing"));
    }

    #[test]
    fn from_value_accepts_legacy_array() {
        let layout = PanelLayout::from_value(&json!(["serves", "jobs"]));
        assert_eq!(layout.order, ["serves", "jobs"]);
        assert!(layout.hidden.is_empty());
    }

    #[test]
    fn move_and_toggle_panels() {
        let mut layout = PanelLayout::default();
        layout.ensure_order(DASHBOARD_PANELS);
        assert!(layout.move_panel("jobs", -1, DASHBOARD_PANELS));
        assert_eq!(layout.order[0], "jobs");
        assert!(!layout.toggle_hidden("jobs"));
        assert_eq!(layout.hidden, ["jobs"]);
        assert!(layout.toggle_hidden("jobs"));
        assert!(layout.hidden.is_empty());
    }

    #[test]
    fn remote_layout_roundtrip() {
        let mut root = json!({});
        store_remote_layout(
            &mut root,
            "local",
            &RemotesLayout {
                order: vec!["b".into(), "a".into()],
                hidden: vec!["a".into()],
            },
        );
        let loaded = load_remote_layout(&root, "local");
        assert_eq!(loaded.order, ["b", "a"]);
        assert_eq!(loaded.hidden, ["a"]);
        assert_eq!(backend_key(""), "local");
        assert_eq!(backend_key("drive-rc"), "drive-rc");
    }
}
