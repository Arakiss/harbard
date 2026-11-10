//! The view's state, and the expressions and conditions that read it.
//!
//! This is the narrow subset of json-render 0.22 a view may use, with its
//! exact semantics: JSON-pointer-like paths, `$state` / `$item` / `$index` /
//! `$template` / `$cond`, first-comparator-wins conditions, JavaScript
//! truthiness and strict equality, and the split of a repeat's `visible`
//! into a container gate and a per-item filter.
//!
//! `None` stands for JavaScript's `undefined` (a path that leads nowhere),
//! which is not the same as `null`.

use serde_json::{Map, Value, json};

/// What an expression can read: the state, and inside a repeat the item.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub state: &'a Value,
    pub item: Option<&'a Value>,
    pub index: Option<usize>,
}

impl<'a> Ctx<'a> {
    pub fn new(state: &'a Value) -> Self {
        Ctx { state, item: None, index: None }
    }

    pub fn with_item(self, item: &'a Value, index: usize) -> Self {
        Ctx { item: Some(item), index: Some(index), ..self }
    }
}

fn segments(path: &str) -> Vec<String> {
    let raw = path.strip_prefix('/').unwrap_or(path);
    raw.split('/').map(|s| s.replace("~1", "/").replace("~0", "~")).collect()
}

/// `getByPath`: `""` and `"/"` are the root; numeric segments index arrays.
pub fn get<'v>(root: &'v Value, path: &str) -> Option<&'v Value> {
    if path.is_empty() || path == "/" {
        return Some(root);
    }
    let mut current = root;
    for seg in segments(path) {
        current = match current {
            Value::Array(items) => items.get(js_parse_int(&seg)?)?,
            Value::Object(fields) => fields.get(&seg)?,
            _ => return None,
        };
    }
    Some(current)
}

/// JavaScript's `parseInt(s, 10)` as an array index: leading digits only.
fn js_parse_int(s: &str) -> Option<usize> {
    let t = s.trim_start();
    let digits: String = t.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

fn numeric(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn dense(items: &[Value], seg: &str) -> Option<usize> {
    if !numeric(seg) {
        return None;
    }
    seg.parse::<usize>().ok().filter(|i| *i <= items.len())
}

/// `setByPath`: creates the objects and arrays on the way.
pub fn set(root: &mut Value, path: &str, value: Value) {
    let segs = segments(path);
    if segs.is_empty() || segs.iter().any(|s| s == "__proto__") {
        return;
    }
    let mut current = root;
    for i in 0..segs.len() - 1 {
        let seg = &segs[i];
        let next_numeric = numeric(&segs[i + 1]) || segs[i + 1] == "-";
        let fresh = || if next_numeric { json!([]) } else { json!({}) };
        current = match current {
            Value::Array(items) => {
                let Some(index) = dense(items, seg) else { return };
                if index == items.len() {
                    items.push(fresh());
                } else if !items[index].is_object() && !items[index].is_array() {
                    items[index] = fresh();
                }
                &mut items[index]
            }
            Value::Object(fields) => {
                let slot = fields.entry(seg.clone()).or_insert_with(fresh);
                if !slot.is_object() && !slot.is_array() {
                    *slot = fresh();
                }
                slot
            }
            _ => return,
        };
    }
    let last = &segs[segs.len() - 1];
    match current {
        Value::Array(items) => {
            if last == "-" {
                items.push(value);
            } else if let Some(index) = dense(items, last) {
                if index == items.len() {
                    items.push(value);
                } else {
                    items[index] = value;
                }
            }
        }
        Value::Object(fields) => {
            fields.insert(last.clone(), value);
        }
        _ => {}
    }
}

/// JavaScript's `Boolean(value)`.
pub fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_)) | Some(Value::Object(_)) => true,
    }
}

/// JavaScript's `===`: scalars by value, arrays and objects by identity.
fn strict_eq(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x == y,
        (Some(Value::Number(x)), Some(Value::Number(y))) => x.as_f64() == y.as_f64(),
        (Some(Value::String(x)), Some(Value::String(y))) => x == y,
        (Some(x @ Value::Array(_)), Some(y @ Value::Array(_))) => std::ptr::eq(x, y),
        (Some(x @ Value::Object(_)), Some(y @ Value::Object(_))) => std::ptr::eq(x, y),
        _ => false,
    }
}

/// A comparison operand: a literal, or `{"$state": "/p"}`.
fn operand<'a>(value: &'a Value, ctx: &Ctx<'a>) -> Option<&'a Value> {
    if let Some(Value::String(path)) = value.as_object().and_then(|o| o.get("$state")) {
        return get(ctx.state, path);
    }
    Some(value)
}

fn source<'a>(cond: &'a Map<String, Value>, ctx: &Ctx<'a>) -> Option<&'a Value> {
    if cond.contains_key("$index") {
        return None;
    }
    if let Some(Value::String(path)) = cond.get("$item") {
        let item = ctx.item?;
        return if path.is_empty() { Some(item) } else { get(item, path) };
    }
    get(ctx.state, cond.get("$state").and_then(Value::as_str).unwrap_or_default())
}

fn as_number(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64)
}

/// One condition: a source, then the first comparator present.
fn single(cond: &Map<String, Value>, ctx: &Ctx<'_>) -> bool {
    let index_value = ctx.index.map(|i| json!(i));
    let value: Option<&Value> = if cond.contains_key("$index") { index_value.as_ref() } else { source(cond, ctx) };
    let numbers = |rhs: &Value, cmp: fn(f64, f64) -> bool| match (as_number(value), as_number(operand(rhs, ctx))) {
        (Some(a), Some(b)) => cmp(a, b),
        _ => false,
    };
    let result = if let Some(rhs) = cond.get("eq") {
        strict_eq(value, operand(rhs, ctx))
    } else if let Some(rhs) = cond.get("neq") {
        !strict_eq(value, operand(rhs, ctx))
    } else if let Some(rhs) = cond.get("gt") {
        numbers(rhs, |a, b| a > b)
    } else if let Some(rhs) = cond.get("gte") {
        numbers(rhs, |a, b| a >= b)
    } else if let Some(rhs) = cond.get("lt") {
        numbers(rhs, |a, b| a < b)
    } else if let Some(rhs) = cond.get("lte") {
        numbers(rhs, |a, b| a <= b)
    } else {
        truthy(value)
    };
    if cond.get("not") == Some(&Value::Bool(true)) { !result } else { result }
}

/// `evaluateVisibility`: `None` (no condition) is visible.
pub fn visible(cond: Option<&Value>, ctx: &Ctx<'_>) -> bool {
    match cond {
        None => true,
        Some(Value::Bool(b)) => *b,
        Some(Value::Array(all)) => all.iter().all(|c| c.as_object().is_some_and(|c| single(c, ctx))),
        Some(Value::Object(o)) => {
            if let Some(Value::Array(all)) = o.get("$and") {
                all.iter().all(|c| visible(Some(c), ctx))
            } else if let Some(Value::Array(any)) = o.get("$or") {
                any.iter().any(|c| visible(Some(c), ctx))
            } else {
                single(o, ctx)
            }
        }
        Some(_) => false,
    }
}

/// Whether a condition reads the repeat item or index.
pub fn uses_item(cond: &Value) -> bool {
    match cond {
        Value::Array(all) => all.iter().any(uses_item),
        Value::Object(o) => {
            o.contains_key("$item")
                || o.contains_key("$index")
                || o.get("$and").and_then(Value::as_array).is_some_and(|a| a.iter().any(uses_item))
                || o.get("$or").and_then(Value::as_array).is_some_and(|a| a.iter().any(uses_item))
        }
        _ => false,
    }
}

/// `splitRepeatVisibility`: (container gate, per-item filter).
pub fn split_repeat(cond: Option<&Value>) -> (Option<Value>, Option<Value>) {
    let Some(cond) = cond else { return (None, None) };
    if !uses_item(cond) {
        return (Some(cond.clone()), None);
    }
    let partition = |parts: &Vec<Value>| {
        let (item, container): (Vec<Value>, Vec<Value>) = parts.iter().cloned().partition(uses_item);
        let wrap = |v: Vec<Value>| (!v.is_empty()).then(|| json!({ "$and": v }));
        (wrap(container), wrap(item))
    };
    match cond {
        Value::Array(parts) => partition(parts),
        Value::Object(o) if o.contains_key("$and") => match o.get("$and") {
            Some(Value::Array(parts)) => partition(parts),
            _ => (None, Some(cond.clone())),
        },
        _ => (None, Some(cond.clone())),
    }
}

/// JavaScript's `String(value)` for what a template prints.
pub fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else if let Some(u) = n.as_u64() {
                u.to_string()
            } else {
                let f = n.as_f64().unwrap_or(0.0);
                if f.fract() == 0.0 && f.abs() < 1e21 { format!("{f:.0}") } else { f.to_string() }
            }
        }
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(|v| if v.is_null() { String::new() } else { js_string(v) })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// `${x}` holes: `/x` reads state; `x` reads the item first, then state `/x`.
pub fn template(text: &str, ctx: &Ctx<'_>) -> String {
    let shown = |v: Option<&Value>| match v {
        None | Some(Value::Null) => None,
        Some(v) => Some(js_string(v)),
    };
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("${") {
        let after = &rest[at + 2..];
        match after.find('}') {
            Some(end) if end > 0 => {
                out.push_str(&rest[..at]);
                let path = &after[..end];
                let value = if path.starts_with('/') {
                    shown(get(ctx.state, path))
                } else {
                    ctx.item
                        .and_then(|item| shown(get(item, path)))
                        .or_else(|| shown(get(ctx.state, &format!("/{path}"))))
                };
                out.push_str(&value.unwrap_or_default());
                rest = &after[end + 1..];
            }
            _ => {
                out.push_str(&rest[..at + 1]);
                rest = &rest[at + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `resolvePropValue` for one prop value. `None` is `undefined`.
pub fn resolve(value: &Value, ctx: &Ctx<'_>) -> Option<Value> {
    let Value::Object(o) = value else {
        return Some(value.clone());
    };
    if let Some(Value::String(path)) = o.get("$state") {
        return get(ctx.state, path).cloned();
    }
    if let Some(Value::String(path)) = o.get("$item") {
        let item = ctx.item?;
        return if path.is_empty() { Some(item.clone()) } else { get(item, path).cloned() };
    }
    if o.get("$index") == Some(&Value::Bool(true)) {
        return ctx.index.map(|i| json!(i));
    }
    if let (Some(cond), Some(then), Some(otherwise)) = (o.get("$cond"), o.get("$then"), o.get("$else")) {
        return resolve(if visible(Some(cond), ctx) { then } else { otherwise }, ctx);
    }
    if let Some(Value::String(text)) = o.get("$template") {
        return Some(Value::String(template(text, ctx)));
    }
    Some(value.clone())
}

/// Every prop resolved; an `undefined` prop is left out, as JSON would.
pub fn resolve_props(props: &Map<String, Value>, ctx: &Ctx<'_>) -> Value {
    let mut out = Map::new();
    for (k, v) in props {
        if let Some(r) = resolve(v, ctx) {
            out.insert(k.clone(), r);
        }
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> Value {
        json!({
            "mode": "table",
            "on": true,
            "n": 3,
            "zero": 0,
            "empty": "",
            "rows": [{"name": "a", "n": 1}, {"name": "b", "n": 5}],
            "deep": {"x": {"y": "z"}}
        })
    }

    #[test]
    fn paths_read_like_json_pointers() {
        let s = state();
        assert_eq!(get(&s, "/mode"), Some(&json!("table")));
        assert_eq!(get(&s, "/rows/1/name"), Some(&json!("b")));
        assert_eq!(get(&s, "/deep/x/y"), Some(&json!("z")));
        assert_eq!(get(&s, "/missing"), None);
        assert_eq!(get(&s, "/rows/9"), None);
        assert_eq!(get(&s, "/mode/length"), None);
        assert_eq!(get(&s, ""), Some(&s));
        assert_eq!(get(&s, "/"), Some(&s));
        assert_eq!(get(&json!({"a": [1]}), "a/0"), Some(&json!(1)));
    }

    #[test]
    fn writes_create_what_is_missing() {
        let mut s = json!({});
        set(&mut s, "/a/b", json!(1));
        assert_eq!(s, json!({"a": {"b": 1}}));
        set(&mut s, "/list/0/name", json!("x"));
        assert_eq!(s["list"], json!([{"name": "x"}]));
        set(&mut s, "/a/b", json!("over"));
        assert_eq!(s["a"]["b"], "over");
        set(&mut s, "/a/b/c", json!(true));
        assert_eq!(s["a"]["b"], json!({"c": true}));
        set(&mut s, "/__proto__/x", json!(1));
        assert!(s.get("__proto__").is_none());
        // An index past the end is ignored, as setByPath does.
        set(&mut s, "/list/5", json!(1));
        assert_eq!(s["list"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn the_first_comparator_wins() {
        let s = state();
        let ctx = Ctx::new(&s);
        assert!(visible(Some(&json!({"$state": "/mode", "eq": "table"})), &ctx));
        // eq decides; the lt that would fail is never read.
        assert!(visible(Some(&json!({"$state": "/n", "eq": 3, "lt": 0})), &ctx));
        assert!(!visible(Some(&json!({"$state": "/n", "neq": 3})), &ctx));
        assert!(visible(Some(&json!({"$state": "/n", "gt": 2})), &ctx));
        assert!(visible(Some(&json!({"$state": "/n", "gte": 3})), &ctx));
        assert!(!visible(Some(&json!({"$state": "/n", "lt": 3})), &ctx));
        assert!(visible(Some(&json!({"$state": "/n", "lte": {"$state": "/n"}})), &ctx));
        // Numeric comparisons need numbers on both sides.
        assert!(!visible(Some(&json!({"$state": "/mode", "gt": 0})), &ctx));
        assert!(visible(Some(&json!({"$state": "/n", "eq": 3.0})), &ctx));
    }

    #[test]
    fn truthiness_and_not_follow_javascript() {
        let s = state();
        let ctx = Ctx::new(&s);
        assert!(visible(Some(&json!({"$state": "/on"})), &ctx));
        assert!(!visible(Some(&json!({"$state": "/zero"})), &ctx));
        assert!(!visible(Some(&json!({"$state": "/empty"})), &ctx));
        assert!(!visible(Some(&json!({"$state": "/missing"})), &ctx));
        assert!(visible(Some(&json!({"$state": "/rows"})), &ctx));
        assert!(visible(Some(&json!({"$state": "/missing", "not": true})), &ctx));
        assert!(!visible(Some(&json!({"$state": "/on", "not": true})), &ctx));
        // undefined is not null.
        assert!(!visible(Some(&json!({"$state": "/missing", "eq": null})), &ctx));
        assert!(visible(Some(&json!({"$state": "/missing", "neq": null})), &ctx));
    }

    #[test]
    fn lists_and_and_or_combine() {
        let s = state();
        let ctx = Ctx::new(&s);
        assert!(visible(Some(&json!(true)), &ctx));
        assert!(!visible(Some(&json!(false)), &ctx));
        assert!(visible(None, &ctx));
        assert!(!visible(Some(&json!([{"$state": "/on"}, {"$state": "/zero"}])), &ctx));
        assert!(visible(Some(&json!({"$or": [{"$state": "/zero"}, {"$state": "/on"}]})), &ctx));
        assert!(!visible(Some(&json!({"$and": [{"$state": "/on"}, {"$or": [false, {"$state": "/empty"}]}]})), &ctx));
    }

    #[test]
    fn item_and_index_conditions_read_the_repeat() {
        let s = state();
        let rows = s["rows"].as_array().unwrap();
        let ctx = Ctx::new(&s).with_item(&rows[1], 1);
        assert!(visible(Some(&json!({"$item": "n", "gt": 2})), &ctx));
        assert!(visible(Some(&json!({"$index": true, "eq": 1})), &ctx));
        assert!(visible(Some(&json!({"$item": ""})), &ctx));
        let outside = Ctx::new(&s);
        assert!(!visible(Some(&json!({"$item": "n"})), &outside));
    }

    #[test]
    fn expressions_resolve() {
        let s = state();
        let ctx = Ctx::new(&s);
        assert_eq!(resolve(&json!({"$state": "/mode"}), &ctx), Some(json!("table")));
        assert_eq!(resolve(&json!({"$state": "/missing"}), &ctx), None);
        assert_eq!(resolve(&json!("plain"), &ctx), Some(json!("plain")));
        assert_eq!(
            resolve(&json!({"$cond": {"$state": "/on"}, "$then": "yes", "$else": {"$state": "/n"}}), &ctx),
            Some(json!("yes"))
        );
        assert_eq!(
            resolve(&json!({"$cond": {"$state": "/zero"}, "$then": "yes", "$else": {"$state": "/n"}}), &ctx),
            Some(json!(3))
        );
        let rows = s["rows"].as_array().unwrap();
        let inside = ctx.with_item(&rows[0], 0);
        assert_eq!(resolve(&json!({"$item": "name"}), &inside), Some(json!("a")));
        assert_eq!(resolve(&json!({"$item": ""}), &inside), Some(rows[0].clone()));
        assert_eq!(resolve(&json!({"$index": true}), &inside), Some(json!(0)));
        assert_eq!(resolve(&json!({"$item": "name"}), &ctx), None);
    }

    #[test]
    fn templates_read_item_then_state() {
        let s = state();
        let rows = s["rows"].as_array().unwrap();
        let ctx = Ctx::new(&s).with_item(&rows[1], 1);
        assert_eq!(template("${name} is ${n} of ${/n} in ${mode}", &ctx), "b is 5 of 3 in table");
        assert_eq!(template("${/missing}|${missing}|${}|$x", &ctx), "||${}|$x");
        assert_eq!(template("${/on} ${/rows}", &Ctx::new(&s)), "true [object Object],[object Object]");
        assert_eq!(template("${/deep/x/y}${", &Ctx::new(&s)), "z${");
    }

    #[test]
    fn numbers_print_as_javascript_does() {
        assert_eq!(js_string(&json!(3)), "3");
        assert_eq!(js_string(&json!(3.0)), "3");
        assert_eq!(js_string(&json!(2.5)), "2.5");
        assert_eq!(js_string(&json!([1, null, "x"])), "1,,x");
    }

    #[test]
    fn props_drop_undefined() {
        let s = state();
        let props = json!({"text": {"$state": "/missing"}, "tone": "muted", "n": {"$state": "/n"}});
        let out = resolve_props(props.as_object().unwrap(), &Ctx::new(&s));
        assert_eq!(out, json!({"tone": "muted", "n": 3}));
    }

    #[test]
    fn a_repeat_condition_splits_into_gate_and_filter() {
        let gate = json!({"$state": "/on"});
        let filter = json!({"$item": "n", "gt": 2});
        assert_eq!(split_repeat(Some(&gate)), (Some(gate.clone()), None));
        assert_eq!(split_repeat(Some(&filter)), (None, Some(filter.clone())));
        assert_eq!(
            split_repeat(Some(&json!([gate, filter]))),
            (Some(json!({"$and": [gate]})), Some(json!({"$and": [filter]})))
        );
        assert_eq!(
            split_repeat(Some(&json!({"$and": [gate, filter]}))),
            (Some(json!({"$and": [gate]})), Some(json!({"$and": [filter]})))
        );
        let either = json!({"$or": [gate, filter]});
        assert_eq!(split_repeat(Some(&either)), (None, Some(either.clone())));
        assert_eq!(split_repeat(None), (None, None));
    }

    #[test]
    fn state_references_compare_by_identity() {
        let s = json!({"a": {"x": 1}, "b": {"x": 1}});
        let ctx = Ctx::new(&s);
        assert!(visible(Some(&json!({"$state": "/a", "eq": {"$state": "/a"}})), &ctx));
        assert!(!visible(Some(&json!({"$state": "/a", "eq": {"$state": "/b"}})), &ctx));
    }
}
