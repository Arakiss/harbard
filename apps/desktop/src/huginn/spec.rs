//! A view as the renderer holds it, after the CLI's validator accepted it.
//!
//! The validator (`harbard_cli::huginn::validate`) is the same wall the
//! console ran (`parseView`): same rules, same issue codes. Only a view it
//! accepts is turned into a [`Spec`]; anything else is shown as refused.

use harbard_cli::huginn::{Issue, validate};
use serde_json::{Map, Value};
use std::collections::HashMap;

/// One element of the flat map.
#[derive(Debug, Clone)]
pub struct Element {
    pub kind: String,
    pub props: Map<String, Value>,
    pub children: Vec<String>,
    pub visible: Option<Value>,
    pub repeat: Option<Repeat>,
}

#[derive(Debug, Clone)]
pub struct Repeat {
    pub state_path: String,
    pub key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Spec {
    pub root: String,
    pub elements: HashMap<String, Element>,
    /// The initial state (an object, or `{}` when the view has none).
    pub state: Value,
}

/// The components that send an answer.
pub fn answers(kind: &str) -> bool {
    matches!(kind, "Choice" | "Form")
}

/// The components that only live inside a Form.
pub fn inside_form(kind: &str) -> bool {
    matches!(
        kind,
        "TextField"
            | "NumberField"
            | "SelectField"
            | "RadioField"
            | "CheckboxField"
            | "MultiSelectField"
            | "DateField"
            | "SliderField"
    )
}

/// The props that name a message attachment.
pub fn file_props(kind: &str) -> &'static [&'static str] {
    match kind {
        "Image" => &["file"],
        "BeforeAfter" => &["before", "after"],
        _ => &[],
    }
}

impl Spec {
    /// Validate and read a view. Refusals carry the validator's issues.
    pub fn parse(source: &[u8]) -> Result<Spec, Vec<Issue>> {
        validate(source)?;
        // Accepted by the validator, so the shape below holds.
        let raw: Value = serde_json::from_slice(source).map_err(|_| Vec::new())?;
        let root = raw["root"].as_str().unwrap_or_default().to_string();
        let mut elements = HashMap::new();
        for (key, el) in raw["elements"].as_object().into_iter().flatten() {
            let repeat = el.get("repeat").and_then(|r| {
                Some(Repeat {
                    state_path: r.get("statePath")?.as_str()?.to_string(),
                    key: r.get("key").and_then(Value::as_str).map(str::to_string),
                })
            });
            elements.insert(
                key.clone(),
                Element {
                    kind: el["type"].as_str().unwrap_or_default().to_string(),
                    props: el
                        .get("props")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default(),
                    children: el
                        .get("children")
                        .and_then(Value::as_array)
                        .map(|c| {
                            c.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default(),
                    visible: el.get("visible").cloned(),
                    repeat,
                },
            );
        }
        let state = match raw.get("state") {
            Some(s @ Value::Object(_)) => s.clone(),
            _ => Value::Object(Map::new()),
        };
        Ok(Spec {
            root,
            elements,
            state,
        })
    }

    /// Every element below `key`, depth first in child order.
    pub fn descendants(&self, key: &str) -> Vec<&Element> {
        let mut out = Vec::new();
        if let Some(el) = self.elements.get(key) {
            for child in &el.children {
                if let Some(c) = self.elements.get(child) {
                    out.push(c);
                    out.extend(self.descendants(child));
                }
            }
        }
        out
    }

    /// The file names the view shows itself (`filesNamedBy`), so a message
    /// does not list them twice.
    pub fn files_named(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for el in self.elements.values() {
            for prop in file_props(&el.kind) {
                if let Some(name) = el.props.get(*prop).and_then(Value::as_str)
                    && !out.iter().any(|n| n == name)
                {
                    out.push(name.to_string());
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_valid_view_reads_into_a_tree() {
        let view = json!({
            "root": "a",
            "elements": {
                "a": {"type": "Stack", "props": {}, "children": ["b"], "repeat": {"statePath": "/rows", "key": "id"}},
                "b": {"type": "Text", "props": {"text": {"$item": "name"}}}
            },
            "state": {"rows": [{"id": "x", "name": "one"}]}
        });
        let spec = Spec::parse(view.to_string().as_bytes()).unwrap();
        assert_eq!(spec.root, "a");
        assert_eq!(spec.elements["a"].children, ["b"]);
        let repeat = spec.elements["a"].repeat.as_ref().unwrap();
        assert_eq!(repeat.state_path, "/rows");
        assert_eq!(repeat.key.as_deref(), Some("id"));
        assert_eq!(spec.state["rows"][0]["name"], "one");
    }

    #[test]
    fn the_shared_fixtures_read_or_refuse_as_in_the_console() {
        let valid: Value = serde_json::from_str(include_str!(
            "../../../../packages/huginn/fixtures/valid.json"
        ))
        .unwrap();
        for case in valid.as_array().unwrap() {
            let spec = Spec::parse(case["view"].to_string().as_bytes());
            assert!(spec.is_ok(), "{}", case["name"]);
        }
        let invalid: Value = serde_json::from_str(include_str!(
            "../../../../packages/huginn/fixtures/invalid.json"
        ))
        .unwrap();
        for case in invalid.as_array().unwrap() {
            let issues = Spec::parse(case["view"].to_string().as_bytes()).unwrap_err();
            assert!(
                issues.iter().any(|i| i.code == case["expect"]),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn a_refused_view_keeps_the_validator_codes() {
        let issues = Spec::parse(br#"{"root":"a","elements":{"a":{"type":"Nope"}}}"#).unwrap_err();
        assert_eq!(issues[0].code, "unknown_component");
        assert_eq!(Spec::parse(b"nope").unwrap_err()[0].code, "invalid_json");
    }

    #[test]
    fn the_files_a_view_shows_are_named() {
        let view = json!({"root":"a","elements":{
            "a":{"type":"Stack","props":{},"children":["b","c"]},
            "b":{"type":"Image","props":{"file":"x.png","alt":"x"}},
            "c":{"type":"BeforeAfter","props":{"before":"x.png","after":"y.png"}}}});
        let mut files = Spec::parse(view.to_string().as_bytes())
            .unwrap()
            .files_named();
        files.sort();
        assert_eq!(files, ["x.png", "y.png"]);
    }
}
