//! The shape a report takes on the bus (F5).
//!
//! Free-form reports cannot be compared: nothing tells an agent what a report
//! looks like, so each invents one.
//!
//! The convention is two rules, and it is chosen so it pays for itself rather
//! than needing discipline:
//!
//!   line 1   a summary, short enough to survive a preview
//!   after    `field: value` lines, one fact each
//!
//! The payment is [`summary_of`]. A brief clips any quoted text to 140 bytes,
//! which is why nine notes in ten arrive cut mid-word; a note in this shape is
//! previewed by its own first line instead, so the author decides what the
//! reader sees rather than the byte counter. Nothing is rejected for not
//! following it — an unstructured note previews exactly as it always did.

/// Bytes of first line that still count as a summary rather than a paragraph.
///
/// Matches the brief's own fragment budget: a summary longer than the preview
/// would be clipped again, which defeats the point of writing one.
pub const SUMMARY_BYTES: usize = 140;

/// True when `line` reads as `field: value` — a lowercase-ish key, a colon,
/// and something after it.
///
/// Deliberately narrow. Prose containing a colon ("Cuellos: (1) …") must NOT
/// be mistaken for structure, so the key may not contain spaces beyond a single
/// internal one and must be short.
fn is_field_line(line: &str) -> bool {
    let Some((key, value)) = line.split_once(':') else {
        return false;
    };
    let key = key.trim();
    if key.is_empty() || key.len() > 24 || value.trim().is_empty() {
        return false;
    }
    key.split(' ').count() <= 2
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == ' ')
}

/// The line to preview a note by, when the author gave one.
///
/// The rule is deliberately NOT "detect a structured report". A first attempt
/// classified `field: value` lines, and it read "Cuellos: (1) 0106 de Quicio…"
/// — ordinary prose with a colon — as structure. Guessing at a shape is how a
/// preview starts lying about what a note says.
///
/// So: a note whose FIRST line is short and which has more below it is
/// previewed by that first line. Nothing is inferred about the rest. This helps
/// every multi-line note, not only reports; the report convention simply
/// guarantees the outcome by construction. A single-paragraph note is clipped
/// by bytes exactly as it always was.
pub fn summary_of(text: &str) -> Option<&str> {
    let mut lines = text.lines();
    let first = lines.next()?.trim();
    if first.is_empty() || first.len() > SUMMARY_BYTES {
        return None;
    }
    // Something must follow, or the "summary" is simply the whole note and the
    // existing clip already shows it.
    lines.any(|l| !l.trim().is_empty()).then_some(first)
}

/// Compose a report body from a summary and `key=value` pairs, in the order the
/// author gave them.
///
/// Returns the field that could not be read, so the CLI can refuse precisely
/// rather than writing a note whose structure is quietly half-applied.
pub fn compose(summary: &str, fields: &[String]) -> Result<String, String> {
    let summary = summary.trim();
    if summary.is_empty() {
        return Err("a report needs a summary line".to_string());
    }
    if summary.len() > SUMMARY_BYTES {
        return Err(format!(
            "the summary is {} bytes; keep it under {SUMMARY_BYTES} so it survives a preview intact",
            summary.len()
        ));
    }
    if summary.contains('\n') {
        return Err("the summary is one line; put the rest in --field".to_string());
    }
    let mut out = String::from(summary);
    for raw in fields {
        let Some((key, value)) = raw.split_once('=') else {
            return Err(format!("expected --field key=value, got \"{raw}\""));
        };
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() || value.is_empty() {
            return Err(format!("expected --field key=value, got \"{raw}\""));
        }
        let line = format!("{key}: {value}");
        if !is_field_line(&line) {
            return Err(format!(
                "\"{key}\" is not a usable field name: one or two words, letters, digits, - or _"
            ));
        }
        out.push('\n');
        out.push_str(&line);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_is_previewed_by_its_own_summary() {
        let body = "group builder closed, waiting on 0106\nstatus: blocked\neta: 90m";
        assert_eq!(
            summary_of(body),
            Some("group builder closed, waiting on 0106")
        );
    }

    #[test]
    fn a_paragraph_is_still_clipped_by_bytes() {
        // One long line and nothing under it: there is no summary to show, and
        // the existing byte clip is already the whole answer.
        let body = "Tizne — group builder. Cerrado: ruta separada para grupo vs wizard, árbol buscable con raíz, matrices intermedias y filiales, validación de grafo.";
        assert_eq!(summary_of(body), None);
    }

    #[test]
    fn a_lone_line_is_not_a_summary() {
        // Nothing follows it, so it is the note, not a summary of one.
        assert_eq!(summary_of("just a normal short note"), None);
    }

    #[test]
    fn any_multi_line_note_gets_its_first_line_back() {
        // Not only reports: an author who wrote a short opening line meant it
        // to be read, and the byte clip used to cut it mid-word anyway.
        let body = "heads up: migration 0107 lands tonight\n\nthe long explanation follows and it is far past the preview budget";
        assert_eq!(
            summary_of(body),
            Some("heads up: migration 0107 lands tonight")
        );
    }

    #[test]
    fn a_summary_too_long_to_survive_a_preview_is_not_one() {
        let long = "x".repeat(SUMMARY_BYTES + 1);
        let body = format!("{long}\nstatus: ok");
        assert_eq!(summary_of(&body), None);
    }

    #[test]
    fn composing_puts_the_summary_first_and_one_fact_per_line() {
        let out = compose(
            "group builder closed, waiting on 0106",
            &[
                "status=blocked".to_string(),
                "eta=90m".to_string(),
                "blocker=Quicio has not committed 0106".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(
            out,
            "group builder closed, waiting on 0106\nstatus: blocked\neta: 90m\nblocker: Quicio has not committed 0106"
        );
        // The point of the shape: it survives its own preview.
        assert_eq!(
            summary_of(&out),
            Some("group builder closed, waiting on 0106")
        );
    }

    #[test]
    fn a_malformed_field_is_named_not_swallowed() {
        let err = compose("summary", &["no-equals-sign".to_string()]).unwrap_err();
        assert!(err.contains("no-equals-sign"), "{err}");
        let err = compose("summary", &["a sentence with spaces=v".to_string()]).unwrap_err();
        assert!(err.contains("not a usable field name"), "{err}");
    }

    #[test]
    fn an_oversized_summary_is_refused_with_its_size() {
        let err = compose(&"x".repeat(200), &[]).unwrap_err();
        assert!(err.contains("200 bytes"), "{err}");
    }

    #[test]
    fn a_report_with_no_fields_is_still_a_note() {
        assert_eq!(compose("just this", &[]).unwrap(), "just this");
    }
}
