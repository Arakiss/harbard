//! What goes back to the agent when the operator answers a view, and how an
//! answer on the bus reads again in the view's own words.
//!
//! The text must match the console's byte for byte (`packages/huginn/src/
//! answer.ts`): agents and `read_answer` parse it. An answer never carries a
//! word the agent wrote: an option travels as its number, a field as its id
//! and what the operator typed.

use super::spec::{Spec, answers, inside_form};
use serde_json::Value;

const HEADER: &str = "Answer · ";
/// Answers stored by older builds open with this header and stay readable.
const OLD_HEADER: &str = "Respuesta · ";
const NO_VALUE: &str = "(no answer)";
const ARROW: &str = " → ";
pub const UNCHANGED: &str = "(proposal unchanged)";
const OLD_UNCHANGED: &str = "(propuesta sin cambios)";
pub const YES: &str = "yes";
pub const NO: &str = "no";
/// Longer answers are refused before they are sent (UTF-16 units, as JS counts).
pub const SEND_LIMIT_CHARS: usize = 1800;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerLine {
    pub field: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub file: String,
    pub id: String,
    pub lines: Vec<AnswerLine>,
}

/// One line of an answer as the operator reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescribedLine {
    pub label: String,
    pub value: String,
}

/// Picked options travel as their 1-based numbers, sorted.
pub fn options_answer(indexes: &[usize]) -> String {
    let mut numbers: Vec<usize> = indexes.iter().map(|i| i + 1).collect();
    numbers.sort_unstable();
    if numbers.len() == 1 {
        format!("option {}", numbers[0])
    } else {
        let list: Vec<String> = numbers.iter().map(usize::to_string).collect();
        format!("options {}", list.join(", "))
    }
}

/// JavaScript's `\s`.
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// What the operator typed, on one line, so it cannot forge another answer line.
fn one_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut gap = false;
    for c in text.chars() {
        if js_space(c) {
            gap = true;
        } else {
            if gap && !out.is_empty() {
                out.push(' ');
            }
            gap = false;
            out.push(c);
        }
    }
    out
}

pub fn compose(answer: &Answer) -> String {
    let mut out = format!("{HEADER}{}#{}", answer.file, answer.id);
    for line in &answer.lines {
        let value = one_line(&line.value);
        out.push('\n');
        out.push_str(&line.field);
        out.push_str(ARROW);
        out.push_str(if value.is_empty() { NO_VALUE } else { &value });
    }
    out
}

/// The length JavaScript would measure.
pub fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// serve stores a reply to an agent as `Para <name>: <text>` / `To <name>: <text>`.
fn strip_addressed(note: &str) -> &str {
    let Some(rest) = note
        .strip_prefix("Para ")
        .or_else(|| note.strip_prefix("To "))
    else {
        return note;
    };
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let mut from = 0;
    while let Some(at) = rest[from..].find(": ") {
        let at = from + at;
        if at > line_end {
            break;
        }
        let after = &rest[at + 2..];
        if after.starts_with("Answer · ") || after.starts_with("Respuesta · ") {
            return after;
        }
        from = at + 1;
    }
    note
}

/// `None` for every note that is not an answer to a view.
pub fn read_answer(note: &str) -> Option<Answer> {
    let text = strip_addressed(note);
    let opening = [HEADER, OLD_HEADER]
        .into_iter()
        .find(|h| text.starts_with(h))?;
    let mut lines = text.split('\n');
    let header = lines.next().unwrap_or_default();
    let hash = header.rfind('#')?;
    if hash <= opening.len() {
        return None;
    }
    let body = lines
        .filter_map(|line| {
            let at = line.find(ARROW)?;
            Some(AnswerLine {
                field: line[..at].to_string(),
                value: line[at + ARROW.len()..].to_string(),
            })
        })
        .collect();
    Some(Answer {
        file: header[opening.len()..hash].to_string(),
        id: header[hash + 1..].to_string(),
        lines: body,
    })
}

/// JavaScript's `Number(s)` for a run of digits and spaces.
fn js_number(s: &str) -> Option<i64> {
    let t = s.trim();
    if t.is_empty() {
        return Some(0);
    }
    if !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse().ok()
}

/// `option 2` / `options 1, 3` back to the option texts.
fn picked(value: &str, options: &[String]) -> Option<String> {
    let rest = ["options ", "option ", "opciones ", "opción "]
        .into_iter()
        .find_map(|p| value.strip_prefix(p))?;
    if rest.is_empty()
        || !rest
            .chars()
            .all(|c| c.is_ascii_digit() || c == ',' || c == ' ')
    {
        return None;
    }
    let labels: Option<Vec<&str>> = rest
        .split(", ")
        .map(|n| {
            let n = js_number(n)?;
            let i = usize::try_from(n - 1).ok()?;
            options.get(i).map(String::as_str)
        })
        .collect();
    labels.map(|l| l.join(", "))
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn prop_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(v) => super::expr::js_string(v),
        None => "undefined".into(),
    }
}

/// An answer as its raw ids, for when the view itself is not at hand.
pub fn describe_raw(answer: &Answer) -> Vec<DescribedLine> {
    answer
        .lines
        .iter()
        .map(|l| DescribedLine {
            label: l.field.clone(),
            value: l.value.clone(),
        })
        .collect()
}

/// An answer in the words of its view; ids stand in for anything the view
/// no longer has.
pub fn describe(spec: &Spec, answer: &Answer) -> Vec<DescribedLine> {
    let raw = || describe_raw(answer);
    let Some((owner_key, owner)) = spec.elements.iter().find(|(_, el)| {
        answers(&el.kind) && el.props.get("id").and_then(Value::as_str) == Some(answer.id.as_str())
    }) else {
        return raw();
    };
    if owner.kind == "Choice" {
        let options: Vec<String> = owner
            .props
            .get("options")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|o| prop_text(o.get("label"))).collect())
            .unwrap_or_default();
        let question = prop_text(owner.props.get("question"));
        return answer
            .lines
            .iter()
            .map(|l| DescribedLine {
                label: question.clone(),
                value: picked(&l.value, &options).unwrap_or_else(|| l.value.clone()),
            })
            .collect();
    }
    let fields: Vec<_> = spec
        .descendants(owner_key)
        .into_iter()
        .filter(|el| inside_form(&el.kind))
        .collect();
    answer
        .lines
        .iter()
        .map(|l| {
            let Some(field) = fields
                .iter()
                .find(|f| f.props.get("id").and_then(Value::as_str) == Some(l.field.as_str()))
            else {
                return DescribedLine {
                    label: l.field.clone(),
                    value: l.value.clone(),
                };
            };
            let unchanged = (l.value == UNCHANGED || l.value == OLD_UNCHANGED)
                .then(|| field.props.get("defaultValue").and_then(Value::as_str))
                .flatten();
            let value = match unchanged {
                Some(d) => d.to_string(),
                None => picked(&l.value, &string_list(field.props.get("options")))
                    .unwrap_or_else(|| l.value.clone()),
            };
            DescribedLine {
                label: prop_text(field.props.get("label")),
                value,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn line(field: &str, value: &str) -> AnswerLine {
        AnswerLine {
            field: field.into(),
            value: value.into(),
        }
    }

    #[test]
    fn options_travel_as_sorted_numbers() {
        assert_eq!(options_answer(&[1]), "option 2");
        assert_eq!(options_answer(&[2, 0]), "options 1, 3");
    }

    #[test]
    fn an_answer_composes_byte_for_byte() {
        let answer = Answer {
            file: "plan.huginn".into(),
            id: "go".into(),
            lines: vec![
                line("pick", "option 2"),
                line("why", "  two\n\tlines  "),
                line("note", "   "),
            ],
        };
        assert_eq!(
            compose(&answer),
            "Answer · plan.huginn#go\npick → option 2\nwhy → two lines\nnote → (no answer)"
        );
    }

    #[test]
    fn an_answer_reads_back() {
        let text = "Answer · plan.huginn#go\npick → option 2\nnot a line\nwhy → a → b";
        let answer = read_answer(text).unwrap();
        assert_eq!(answer.file, "plan.huginn");
        assert_eq!(answer.id, "go");
        assert_eq!(
            answer.lines,
            vec![line("pick", "option 2"), line("why", "a → b")]
        );
        // serve's address prefix and the old header.
        let addressed = read_answer("To lead: Answer · a#b.huginn#x\nf → 1").unwrap();
        assert_eq!(
            (addressed.file.as_str(), addressed.id.as_str()),
            ("a#b.huginn", "x")
        );
        assert!(read_answer("Para Ana: Respuesta · v.huginn#q").is_some());
        assert!(read_answer("just a note").is_none());
        assert!(read_answer("Answer · #x").is_none());
        assert!(read_answer("To lead: hi\nAnswer · v.huginn#q").is_none());
    }

    fn spec() -> Spec {
        let view = json!({"root": "s", "elements": {
            "s": {"type": "Stack", "props": {}, "children": ["c", "f"]},
            "c": {"type": "Choice", "props": {"id": "pick", "question": "Which?", "options": [{"label": "A"}, {"label": "B"}, {"label": "C"}]}},
            "f": {"type": "Form", "props": {"id": "form", "title": "Details"}, "children": ["t", "m", "k"]},
            "t": {"type": "TextField", "props": {"id": "name", "label": "Name", "defaultValue": "Ada"}},
            "m": {"type": "MultiSelectField", "props": {"id": "tags", "label": "Tags", "options": ["x", "y", "z"]}},
            "k": {"type": "CheckboxField", "props": {"id": "ok", "label": "Agree"}}
        }});
        Spec::parse(view.to_string().as_bytes()).unwrap()
    }

    #[test]
    fn an_answer_describes_in_the_views_words() {
        let s = spec();
        let choice = Answer {
            file: "v".into(),
            id: "pick".into(),
            lines: vec![line("pick", "options 1, 3")],
        };
        assert_eq!(
            describe(&s, &choice),
            vec![DescribedLine {
                label: "Which?".into(),
                value: "A, C".into()
            }]
        );
        let form = Answer {
            file: "v".into(),
            id: "form".into(),
            lines: vec![
                line("name", UNCHANGED),
                line("tags", "option 2"),
                line("ok", "yes"),
                line("gone", "1"),
            ],
        };
        let words = describe(&s, &form);
        assert_eq!(
            words[0],
            DescribedLine {
                label: "Name".into(),
                value: "Ada".into()
            }
        );
        assert_eq!(
            words[1],
            DescribedLine {
                label: "Tags".into(),
                value: "y".into()
            }
        );
        assert_eq!(
            words[2],
            DescribedLine {
                label: "Agree".into(),
                value: "yes".into()
            }
        );
        assert_eq!(
            words[3],
            DescribedLine {
                label: "gone".into(),
                value: "1".into()
            }
        );
        // Old Spanish answers and out-of-range options.
        let old = Answer {
            file: "v".into(),
            id: "pick".into(),
            lines: vec![line("pick", "opción 2"), line("pick", "option 9")],
        };
        let words = describe(&s, &old);
        assert_eq!(words[0].value, "B");
        assert_eq!(words[1].value, "option 9");
        let unknown = Answer {
            file: "v".into(),
            id: "nope".into(),
            lines: vec![line("a", "b")],
        };
        assert_eq!(
            describe(&s, &unknown),
            vec![DescribedLine {
                label: "a".into(),
                value: "b".into()
            }]
        );
    }

    #[test]
    fn length_counts_like_javascript() {
        assert_eq!(js_len("→"), 1);
        assert_eq!(js_len("😀"), 2);
    }
}
