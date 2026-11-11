//! The text an agent reads in `harbard huginn catalog`, written from the
//! catalog so the two cannot disagree.
//!
//! `guide.md` beside this file is the same text, checked in so a change to
//! the catalog shows what agents will read. A test fails when it is out of
//! date; `just huginn-guide` writes it again.

use serde_json::{Map, Value};

use super::{Component, catalog};

const HEADER: &str = r#"# Huginn: show the operator a view instead of a sentence

A view is a JSON file with the extension .huginn, attached to a note. The note's
text stays the message; the view renders under it in the app.

  harbard huginn check view.huginn
  harbard note "..." --attach view.huginn --attach chart.png

{ "root": "card", "elements": {
    "card":  { "type": "Card", "props": { "title": "Deploy" }, "children": ["state"] },
    "state": { "type": "Metric", "props": { "label": "Errors", "value": "0" } } } }

Rules: root plus a flat map of elements; each child has one parent. An unknown prop
is refused. There are no actions and no watchers.
Answers: Choice, Form and each *Field need an id. The note you get holds no text of
yours, only ids and numbers: "plan → option 2" is the second option of Choice plan;
"reason → text" is what the operator typed; "(proposal unchanged)" means your
defaultValue was left alone. Options count from 1.
Files: Image and BeforeAfter name files attached in the same command.
Limits: @elements@ elements, @depth@ levels, @bytes@ bytes.

State is optional and stays inside the view:
  { "root": "s", "state": { "env": "dev", "rows": [{ "name": "api", "ok": true }] },
    "elements": { "s": { "type": "Stack", "props": {}, "children": ["pick", "list"] },
      "pick": { "type": "Segmented", "props": { "state": "/env", "options": [
        { "label": "Dev", "value": "dev" }, { "label": "Prod", "value": "prod" }] } },
      "list": { "type": "Stack", "props": {}, "children": ["row"],
        "repeat": { "statePath": "/rows", "key": "name" },
        "visible": { "$item": "ok", "eq": true } },
      "row": { "type": "Text", "props": { "text": { "$template": "${name} en ${/env}" } } } } }
A prop may be a whole expression: $state, $item, $index, $template, or
$cond/$then/$else. Not inside lists, and not in control, answer or file props.
visible hides an element: { "$state": "/x", "eq": 1 }, gt, gte, lt, lte, neq, not,
$and, $or. On a repeat it filters items. A *Field with state writes its value there,
so one field can show another. One repeat per branch, @repeatElements@ elements inside,
@repeatItems@ items. Answer and control components cannot repeat.

Use a view when the shape carries meaning: a table, a figure, a decision. A short
answer stays text.
"#;

/// A schema as an agent reads a type: `"a"|"b"`, `string[]`, `{ x: int }`.
fn type_of(schema: &Value) -> String {
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return values
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("|");
    }
    if let Some(options) = schema.get("anyOf").and_then(Value::as_array) {
        return options.iter().map(type_of).collect::<Vec<_>>().join("|");
    }
    let ty = schema.get("type").and_then(Value::as_str);
    match ty {
        Some("array") => {
            let item = schema.get("items").map_or_else(|| "any".into(), type_of);
            format!("{item}[]")
        }
        Some("object") => format!("{{ {} }}", props_line(schema)),
        _ => match schema.get("x-huginn-format").and_then(Value::as_str) {
            Some(format) => format.to_string(),
            None if ty == Some("integer") => "int".into(),
            None => ty.unwrap_or("any").to_string(),
        },
    }
}

/// `name: type` for each property, `name?:` when it may be left out.
fn props_line(schema: &Value) -> String {
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let empty = Map::new();
    schema
        .get("properties")
        .and_then(Value::as_object)
        .unwrap_or(&empty)
        .iter()
        .map(|(name, prop)| {
            let optional = if required.contains(&name.as_str()) {
                ""
            } else {
                "?"
            };
            format!("{name}{optional}: {}", type_of(prop))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn component_line(name: &str, component: &Component) -> String {
    let children = match component.children {
        super::ChildRule::Arity(super::Arity::None) => "",
        _ => " +children",
    };
    format!(
        "  {name} {{ {} }}{children}\n    {}",
        props_line(&component.props),
        component.description
    )
}

/// The guide: how a view is written, then every component by family, in
/// the catalog's order.
pub fn agent_guide() -> String {
    let catalog = catalog();
    let limits = &catalog.limits;
    let header = HEADER
        .replace("@elements@", &limits.elements.to_string())
        .replace("@depth@", &limits.depth.to_string())
        .replace("@bytes@", &limits.bytes.to_string())
        .replace("@repeatElements@", &limits.repeat_elements.to_string())
        .replace("@repeatItems@", &limits.repeat_items.to_string());
    let sections: Vec<String> = catalog
        .families
        .iter()
        .map(|(family, about)| {
            let lines: Vec<String> = catalog
                .order
                .iter()
                .filter_map(|name| Some((name, catalog.components.get(name)?)))
                .filter(|(_, component)| &component.family == family)
                .map(|(name, component)| component_line(name, component))
                .collect();
            let about = about.as_str().unwrap_or_default();
            format!("## {family}: {about}\n{}", lines.join("\n"))
        })
        .collect();
    format!("{header}\n{}\n", sections.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHECKED_IN: &str = include_str!("guide.md");

    /// `HUGINN_BLESS=1` writes the guide instead of comparing it
    /// (`just huginn-guide`).
    #[test]
    fn guide_md_is_the_guide_as_the_catalog_stands() {
        let guide = agent_guide();
        if std::env::var_os("HUGINN_BLESS").is_some() {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/huginn/guide.md");
            std::fs::write(path, &guide).expect("write guide.md");
            return;
        }
        assert!(
            guide == CHECKED_IN,
            "crates/harbard-cli/src/huginn/guide.md is out of date: run `just huginn-guide`"
        );
    }

    #[test]
    fn a_type_reads_as_an_agent_writes_it() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "rows": {"type": "array", "items": {"type": "array", "items": {"anyOf": [{"type": "string"}, {"type": "null"}]}}},
                "level": {"type": "integer"},
                "href": {"type": "string", "x-huginn-format": "http_url"},
                "tone": {"type": "string", "enum": ["a", "b"]}
            },
            "required": ["level"]
        });
        assert_eq!(
            props_line(&schema),
            r#"rows?: string|null[][], level: int, href?: http_url, tone?: "a"|"b""#
        );
    }
}
