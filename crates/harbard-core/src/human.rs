//! Messages v2: the human line, the technical body and the jargon warning.
//!
//! An agent message keeps `<LABEL> · <name> · <sentence>` on its first line.
//! Under v2 that sentence is written for the operator, in their language: what
//! changed for them or what they must decide, with no commit hashes, branch
//! names, review codes or engineering words. The technical detail follows
//! after one blank line, in any language, for the lead.
//!
//! A message with no blank line after its first line is v1 and reads as it
//! always did: the first line only, no body.
//!
//! Everything here is lexical: it decides nothing about the work, only how a
//! line reads.

/// Longest technical body kept, in characters. The protocol asks for
/// messages under 1,200; this leaves room and still bounds the wire.
pub const BODY_CAP: usize = 4_000;
/// Absolute paths kept from one message body.
pub const PATHS_KEPT: usize = 16;
/// Longest path token considered.
const PATH_CAP: usize = 400;

/// Engineering words that never belong in the human line.
const JARGON_WORDS: [&str; 7] = [
    "rebase", "merge", "frozen", "gate", "sha", "worktree", "commit",
];

/// The technical body of a v2 message: everything after the first non-empty
/// line, when a blank line separates the two. `None` for v1 (no blank line)
/// and for a message with nothing after the first line. Capped at
/// [`BODY_CAP`]; tokens that read as credentials are masked.
pub fn technical_body(text: &str) -> Option<String> {
    let mut lines = text.lines();
    // Skip leading blank lines, then the first line itself.
    lines.by_ref().find(|l| !l.trim().is_empty())?;
    let next = lines.next()?;
    if !next.trim().is_empty() {
        return None;
    }
    let rest: Vec<&str> = lines.collect();
    let body = rest.join("\n");
    let body = body.trim_matches(|c: char| c == '\n' || c == '\r');
    if body.trim().is_empty() {
        return None;
    }
    Some(cap(&mask_secrets(body), BODY_CAP))
}

/// A note's text as the workspace shows it, never cut short of [`BODY_CAP`]:
/// its first line in full, and what follows it. A v2 note (a blank line
/// after the first) keeps its technical body apart, as an agent message
/// does; a v1 note's further lines belong to its text (`more`). What
/// follows the first line has its credentials masked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteText {
    /// The first non-empty line, trimmed.
    pub first: String,
    /// A v1 note's lines after the first.
    pub more: Option<String>,
    /// A v2 note's technical body.
    pub body: Option<String>,
}

/// Split a note into [`NoteText`].
pub fn note_text(text: &str) -> NoteText {
    let mut lines = text.lines();
    let first = lines
        .by_ref()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let first = cap(first, BODY_CAP);
    if let Some(body) = technical_body(text) {
        return NoteText {
            first,
            more: None,
            body: Some(body),
        };
    }
    let rest: Vec<&str> = lines.collect();
    let rest = rest.join("\n");
    let rest = rest.trim();
    let more = (!rest.is_empty()).then(|| cap(&mask_secrets(rest), BODY_CAP));
    NoteText {
        first,
        more,
        body: None,
    }
}

/// The whole of a text the operator wrote, trimmed, up to [`BODY_CAP`]:
/// their notes show as they wrote them, every line.
pub fn whole_text(text: &str) -> String {
    cap(text.trim(), BODY_CAP)
}

/// Mask every credential in a technical body through the one shared
/// filter, [`crate::redact::mask_secrets`]: a token after a path prefix, an
/// `=`, a `:`, a `?`, quotes, a tab or a no-break space is masked too
///. Only the attachment paths `harbard attach` wrote keep their
/// ULID: they are set aside before masking and put back after.
pub fn mask_secrets(text: &str) -> String {
    const OPEN: char = '\u{E000}';
    const CLOSE: char = '\u{E001}';
    let atts = attachment_paths(text);
    let mut held = text.replace([OPEN, CLOSE], "");
    for (i, p) in atts.iter().enumerate() {
        held = held.replace(p.as_str(), &format!("{OPEN}{i}{CLOSE}"));
    }
    let mut out = crate::redact::mask_secrets(&held);
    for (i, p) in atts.iter().enumerate() {
        out = out.replace(&format!("{OPEN}{i}{CLOSE}"), p);
    }
    out
}

/// At most `max` bytes, cut at a character boundary, with `…`.
fn cap(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s[..end].to_string();
    out.push('…');
    out
}

/// A path `harbard attach` wrote: `…/attachments/<ULID>/<name>`, nothing
/// after. Lexical; the only paths a message keeps.
pub fn is_attachment_path(path: &str) -> bool {
    let Some((before, rest)) = path.rsplit_once("/attachments/") else {
        return false;
    };
    let Some((dir, name)) = rest.split_once('/') else {
        return false;
    };
    !before.is_empty()
        && dir.len() == 26
        && dir
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase())
        && !name.is_empty()
        && !name.starts_with('.')
        && name.len() <= 100
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The attachment paths named in `text`.
pub fn attachment_paths(text: &str) -> Vec<String> {
    path_tokens(text)
        .into_iter()
        .filter(|p| is_attachment_path(p))
        .collect()
}

/// Every absolute path (`/…` or `~/…`) named in `text`, in order, without
/// duplicates, at most [`PATHS_KEPT`]. Lexical: nothing is looked at on disk.
pub fn path_tokens(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split(char::is_whitespace) {
        let token = raw
            .trim_start_matches(['`', '\'', '"', '(', '[', '<', '*'])
            .trim_end_matches([
                '`', '\'', '"', ')', ']', '>', ',', ';', '*', '.', ':', '!', '?',
            ]);
        let token = token.split_once(':').map_or(token, |(p, _)| p);
        if !(token.starts_with('/') || token.starts_with("~/"))
            || token.len() < 3
            || token.len() > PATH_CAP
            || token.contains("/../")
            || token.ends_with("/..")
        {
            continue;
        }
        if !out.iter().any(|p| p == token) {
            out.push(token.to_string());
            if out.len() == PATHS_KEPT {
                break;
            }
        }
    }
    out
}

/// Short English function words: a line made of these is English.
/// No "has": it is a Spanish verb form too ("has hecho bien").
const ENGLISH: [&str; 43] = [
    "the", "and", "is", "are", "was", "were", "of", "to", "onto", "into", "with", "for", "this",
    "that", "these", "those", "it", "its", "on", "at", "by", "from", "new", "now", "have", "had",
    "be", "been", "not", "all", "after", "before", "which", "will", "can", "should", "my", "your",
    "we", "they", "there", "here", "but",
];

/// Short Spanish function words, to tell a Spanish line with an English word
/// from an English line.
const SPANISH: [&str; 43] = [
    "el", "la", "los", "las", "de", "del", "que", "y", "en", "un", "una", "por", "para", "con",
    "se", "es", "lo", "su", "sus", "ya", "está", "están", "hay", "sin", "al", "te", "tu", "más",
    "pero", "como", "cuando", "he", "ha", "son", "han", "sí", "también", "puedes", "ahora", "qué",
    "mi", "no", "muy",
];

/// Why a human line reads as jargon, in a stable order:
/// - `sha`: a hexadecimal commit hash of 7 or more characters;
/// - `review_code`: a review code such as `R2-N1`, `H1` or `R2-a`;
/// - `word`: rebase, merge, frozen, gate, SHA, worktree or commit.
///
/// Empty for a plain line in any language.
pub fn jargon(line: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    if has_sha(line) {
        out.push("sha");
    }
    if line
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ':' | '(' | ')' | '.'))
        .any(is_review_code)
    {
        out.push("review_code");
    }
    if words(line).any(|w| is_jargon_word(&w)) {
        out.push("word");
    }
    out
}

/// A run of 7 to 40 hex digits holding at least one digit. All letters is a
/// word (`deadbeef`, `facade`); all digits is a number only when it is short
/// of a SHA's look, so a 7-digit run with no letter is not flagged either.
///
/// Lower-case only, as git prints them: an upper-case hex run is an id of
/// the operator's own domain (a NIF, a CIF, an invoice number), unless the word
/// "commit" or "sha" sits beside it.
fn has_sha(line: &str) -> bool {
    let runs: Vec<&str> = line
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|r| !r.is_empty())
        .collect();
    runs.iter().enumerate().any(|(i, run)| {
        let hexlike = (7..=40).contains(&run.len())
            && run.bytes().all(|b| b.is_ascii_hexdigit())
            && run.bytes().any(|b| b.is_ascii_digit())
            && run.bytes().any(|b| b.is_ascii_alphabetic());
        if !hexlike {
            return false;
        }
        let lower = run.bytes().all(|b| !b.is_ascii_uppercase());
        let named = [i.checked_sub(1), Some(i + 1)]
            .into_iter()
            .flatten()
            .filter_map(|j| runs.get(j))
            .any(|w| matches!(w.to_ascii_lowercase().as_str(), "commit" | "sha"));
        lower || named
    })
}

/// Ids of the operator's domain that look like review codes: a quarter (`Q3`,
/// `T2`), a terminal (`T4`), a language level (`B2`), a tax id with eight or
/// more digits.
fn domain_code(token: &str) -> bool {
    let b = token.as_bytes();
    let caps = b.iter().take_while(|c| c.is_ascii_uppercase()).count();
    let rest = &b[caps..];
    let digits = rest.iter().all(|c| c.is_ascii_digit()) && !rest.is_empty();
    (b.len() == 2 && matches!(b[0], b'Q' | b'T') && (b'1'..=b'4').contains(&b[1]))
        || (b.len() == 2 && matches!(b[0], b'A' | b'B' | b'C') && matches!(b[1], b'1' | b'2'))
        || ((1..=2).contains(&caps) && digits && rest.len() >= 8)
}

/// `H1`, `N6`, `R2-N1`, `R2-a`, `M4a`: one to three capitals, digits, an
/// optional lower-case letter, and more such parts after hyphens.
fn is_review_code(token: &str) -> bool {
    let token = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
    if token.is_empty() || token.len() > 16 || domain_code(token) {
        return false;
    }
    let mut parts = token.split('-');
    let Some(head) = parts.next() else {
        return false;
    };
    if !code_part(head, true) {
        return false;
    }
    parts.all(|p| code_part(p, false))
}

fn code_part(part: &str, head: bool) -> bool {
    let bytes = part.as_bytes();
    let caps = bytes.iter().take_while(|b| b.is_ascii_uppercase()).count();
    let rest = &bytes[caps..];
    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    let tail = &rest[digits..];
    let tail_ok = tail.is_empty() || (tail.len() == 1 && tail[0].is_ascii_lowercase());
    if head {
        (1..=3).contains(&caps) && (1..=3).contains(&digits) && tail_ok
    } else {
        // After a hyphen: `N1`, `a`, `2`.
        !part.is_empty()
            && caps <= 3
            && digits <= 3
            && tail_ok
            && (caps + digits > 0 || tail.len() == 1)
    }
}

fn words(line: &str) -> impl Iterator<Item = String> + '_ {
    line.split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
}

/// Mostly English: at least two English function words and more of them
/// than Spanish ones, and no Spanish letters (á, é, í, ó, ú, ñ, ü, ¿, ¡).
pub(crate) fn is_english(line: &str) -> bool {
    if line.chars().any(|c| "áéíóúñüÁÉÍÓÚÑÜ¿¡".contains(c)) {
        return false;
    }
    let (mut en, mut es) = (0, 0);
    for w in words(line) {
        if ENGLISH.contains(&w.as_str()) {
            en += 1;
        }
        if SPANISH.contains(&w.as_str()) {
            es += 1;
        }
    }
    en >= 2 && en > es
}

fn is_jargon_word(word: &str) -> bool {
    JARGON_WORDS.iter().any(|j| {
        word.strip_prefix(j).is_some_and(|rest| {
            matches!(
                rest,
                "" | "s" | "d" | "ed" | "es" | "ing" | "ear" | "eado" | "eada"
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_line_after_the_first_splits_the_human_line_from_the_body() {
        let text = "DONE · harbard-view · Ya puedes ver las capturas.\n\nRebased onto main; head 1582702.\nTests green.";
        assert_eq!(
            technical_body(text).as_deref(),
            Some("Rebased onto main; head 1582702.\nTests green.")
        );
        // Leading blank lines before the first line do not count.
        assert_eq!(
            technical_body("\n\nDONE · a · Hecho.\n\n  detalle").as_deref(),
            Some("  detalle")
        );
    }

    #[test]
    fn a_v1_message_has_no_body() {
        // No blank line: the second line is part of a v1 message, as today.
        assert_eq!(technical_body("DONE · a · Hecho.\nmore detail"), None);
        assert_eq!(technical_body("DONE · a · Hecho."), None);
        assert_eq!(technical_body("DONE · a · Hecho.\n\n   \n"), None);
        assert_eq!(technical_body(""), None);
    }

    #[test]
    fn the_body_is_capped_and_masks_credentials_but_keeps_paths() {
        let long = format!("DONE · a · Hecho.\n\n{}", "x".repeat(BODY_CAP + 50));
        let body = technical_body(&long).unwrap();
        assert_eq!(body.chars().count(), BODY_CAP + 1);
        assert!(is_attachment_path(
            "/h/.harbard/attachments/01K9Z8Y7X6W5V4T3S2R1Q0P9N8/antes.png"
        ));
        for p in [
            "/h/attachments/x/antes.png",
            "/h/.harbard/attachments/01K9Z8Y7X6W5V4T3S2R1Q0P9N8/a/b.png",
            "/Users/p/otro.md",
        ] {
            assert!(!is_attachment_path(p), "{p}");
        }
        let body = technical_body(concat!(
            "DONE · a · Hecho.\n\nclave ghp_abcd",
            "efghijklmnop1234 en /Users/p/.harbard/attachments/01K9Z8Y7X6W5V4T3S2R1Q0P9N8/antes.png"
        ))
        .unwrap();
        assert!(!body.contains("ghp_"), "{body}");
        assert!(body.contains("•••"));
        assert!(
            body.contains("/Users/p/.harbard/attachments/01K9Z8Y7X6W5V4T3S2R1Q0P9N8/antes.png")
        );
    }

    #[test]
    fn path_tokens_are_absolute_trimmed_and_unique() {
        let paths = path_tokens(
            "Mira `/a/b.png`, (~/c/d.md) y /a/b.png. No: ../x, /a/../etc/passwd, relative/x.md",
        );
        assert_eq!(paths, vec!["/a/b.png".to_string(), "~/c/d.md".to_string()]);
    }

    #[test]
    fn each_jargon_trigger_is_caught() {
        assert_eq!(jargon("Listo en fcf628c para que lo pruebes"), vec!["sha"]);
        assert_eq!(
            jargon("He cerrado R2-N1 y la revisión ya está"),
            vec!["review_code"]
        );
        assert_eq!(
            jargon("Corregido H1, ya puedes probarlo"),
            vec!["review_code"]
        );
        assert_eq!(jargon("Quedan R2-a y N6 por cerrar"), vec!["review_code"]);
        assert_eq!(
            jargon("The new console is ready for you to try"),
            Vec::<&str>::new()
        );
        for word in [
            "rebase", "merge", "frozen", "gate", "SHA", "worktree", "commit",
        ] {
            let line = format!("Ya está el {word} de la consola");
            assert_eq!(jargon(&line), vec!["word"], "{line}");
        }
        assert_eq!(jargon("He hecho los commits que faltaban"), vec!["word"]);
        // Today's screenshot line trips three at once.
        assert_eq!(
            jargon("rebased onto robots fcf628c; the new head is 1582702, frozen"),
            vec!["sha", "word"]
        );
    }

    #[test]
    fn a_good_spanish_line_gets_no_chip() {
        for line in [
            "Ya puedes ver las capturas de antes y después de la consola.",
            "Necesito que decidas si la factura va a nombre de la empresa.",
            "He terminado el informe; está en la pestaña de la derecha.",
            "Tus mensajes ahora muestran si alguien los ha leído.",
            "Listo: 1.200 caracteres como máximo, y el 25 de julio es el plazo.",
            "La puerta de entrada ya no se queda congelada.",
        ] {
            assert!(jargon(line).is_empty(), "{line}: {:?}", jargon(line));
        }
    }

    /// No secret survives in a v2 body in
    /// any context, a path prefix included, and an attachment path keeps
    /// its ULID.
    #[test]
    fn the_reviewers_contexts_never_leak_a_secret() {
        let secrets = [
            concat!("ghp_abcd", "efghijklmnop1234567890ABCD"),
            concat!("sk-proj", "-AbCdEf123456GhIjKl7890MnOp"),
            concat!("AKIAIOSF", "ODNN7EXAMPLE"),
            concat!("xoxb-1234", "56789012-abcdefghijklmnop"),
            concat!("eyJhbGc", "iOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.abcDEF123"),
        ];
        let contexts = [
            "token {S}",
            "KEY={S}",
            "\"api\":\"{S}\"",
            "/Users/p/app/.env:1:OPENAI_API_KEY={S}",
            "~/app/.env:3:X={S}",
            "/api/v1/hook?k={S}",
            "(/tmp/x={S})",
            "a\t{S}",
            "a\u{00A0}{S}",
        ];
        let mut leaks = Vec::new();
        for c in contexts {
            for s in secrets {
                let text = c.replace("{S}", s);
                let body = technical_body(&format!("DONE · a · Hecho.\n\n{text}")).unwrap();
                if body.contains(s) {
                    leaks.push(format!("{c:?}: {body}"));
                }
                if attachment_paths(&text).iter().any(|p| p.contains(s)) {
                    leaks.push(format!("path {c:?}"));
                }
            }
        }
        let url = "postgres://admin:Sup3rS3cret!@db.internal/app";
        if technical_body(&format!("DONE · a · Hecho.\n\n{url}")).unwrap() == url {
            leaks.push(format!("low-entropy {url:?}"));
        }
        assert!(leaks.is_empty(), "{} leaks: {leaks:#?}", leaks.len());
        let att = "/h/.harbard/attachments/01K9Z8Y7X6W5V4T3S2R1Q0P9N8/antes.png";
        let body = technical_body(&format!(
            concat!(
                "DONE · a · x.\n\nCaptura {att} y token ghp_abcd",
                "efghijklmnop1234567890ABCD"
            ),
            att = att
        ))
        .unwrap();
        assert!(body.contains(att) && !body.contains("ghp_"), "{body}");
    }

    /// The one probe case the shared filter does not catch yet: a password
    /// glued to its flag (`-pHunter2pass`). redact.rs is directo's; reported
    /// to the lead rather than patched here.
    #[test]
    #[ignore = "redact.rs gap: a password glued to -p; owner harbard-directo"]
    fn a_password_glued_to_its_flag() {
        let c = "mysql -uroot -pHunter2pass";
        let body = technical_body(&format!("DONE · a · Hecho.\n\n{c}")).unwrap();
        assert_ne!(body, c);
    }

    /// The shared matrix (redact::testkit: 41 contexts by 10 secret classes),
    /// over what the workspace serves: the body on the wire as JSON, and as
    /// the "Technical details" text the console renders (HTML-escaped).
    #[test]
    fn the_redact_matrix_over_the_body_json_and_html() {
        use crate::redact::testkit::{REVIEW_TEMPLATES, Rng, review_secret};
        let mut r = Rng(0x5eed_4b0d_c0ff_ee01);
        let mut checked = 0;
        for _ in 0..12 {
            for (name, template) in REVIEW_TEMPLATES {
                let (class, secret) = review_secret(&mut r);
                let text = format!("DONE · a · Hecho.\n\n{}", template.replace("{}", &secret));
                let Some(body) = technical_body(&text) else {
                    continue;
                };
                let json = serde_json::to_string(&body).unwrap();
                let html = body
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace('"', "&quot;");
                for (what, out) in [("body", &body), ("json", &json), ("html", &html)] {
                    assert!(
                        !out.contains(&secret),
                        "{what} leaked {class} in {name}: {out}"
                    );
                }
                checked += 1;
            }
        }
        assert!(checked >= 400, "{checked}");
    }

    /// The operator's own domain is not jargon.
    #[test]
    fn spanish_tax_and_everyday_lines_get_no_chip() {
        for line in [
            "Ya está presentado el modelo 303 del Q3",
            "La declaración del T2 está lista para que la revises",
            "He puesto el NIF de Zooplus DE200164421 en la factura",
            "El CIF B12345678 ya figura en el contrato",
            "Te recojo en la T4 a las seis",
            "Has hecho bien; has dejado todo listo",
            "Necesitas el nivel B2 para el examen",
            "La factura FA2026001 ya está enviada",
        ] {
            assert!(jargon(line).is_empty(), "{line}: {:?}", jargon(line));
        }
        // A real hash still is, and so is one named as a commit.
        assert_eq!(jargon("Integrado en fcf628c"), vec!["sha"]);
        assert_eq!(
            jargon("Integrado en el commit FCF628C"),
            vec!["sha", "word"]
        );
    }

    /// The cap is on bytes, at a character boundary.
    #[test]
    fn the_body_cap_counts_bytes() {
        let text = format!("DONE · a · Hecho.\n\n{}", "𝄞".repeat(BODY_CAP));
        let body = technical_body(&text).unwrap();
        assert!(
            body.len() <= BODY_CAP + '…'.len_utf8(),
            "{} bytes",
            body.len()
        );
        assert!(body.ends_with('…'));
    }

    #[test]
    fn near_misses_are_not_jargon() {
        // All-letter hex words, plain numbers, lower-case versions, words
        // that only start like a jargon word.
        assert!(jargon("La fachada decade y el café 1234567").is_empty());
        assert!(jargon("Versión v2 del protocolo, sin cambios para ti").is_empty());
        assert!(jargon("El gateway de pagos y la shakira de turno").is_empty());
        assert!(!is_review_code("ABCD1"));
        assert!(!is_review_code("R"));
        assert!(!is_review_code("2R"));
        assert!(is_review_code("M4a"));
    }
}
