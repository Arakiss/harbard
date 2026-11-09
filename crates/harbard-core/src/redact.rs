//! The one place harbard masks secrets: in a label (a file name, a Grep
//! pattern, a host, a command word; [`looks_secret`]) and in prose an agent
//! wrote ([`mask_secrets`]). Everything that shows text from a transcript
//! goes through here, so a shape caught once is caught everywhere.
//!
//! [`mask_secrets`] masks only what reads as a credential, piece by piece,
//! and keeps every other word: a message is never masked whole. Its tests
//! include the review's matrix of shapes (`harbard-directo-review.md`):
//! secrets glued to paths, links, compact JSON, backslash paths, Spanish
//! names, table cells, URL userinfo, queries and fragments.

/// Words that mark a secret wherever they appear in a label.
const SECRET_MARKS: [&str; 15] = [
    "token",
    "secret",
    "passw",
    "apikey",
    "api_key",
    "api-key",
    "bearer",
    "authoriz",
    "credential",
    "sk-",
    "sk_",
    "ghp_",
    "github_pat",
    "xox",
    "glpat",
];

/// Could `word` be a credential? Marked by name, an AWS key id, or a run of
/// letters and digits that reads as random rather than as words:
/// - 20 or more with at least three digits;
/// - 16 or more that switch between upper case, lower case and digits on at
///   least three characters in ten (base62 does; camelCase does not);
/// - 24 or more letters of one case with the spread of random text (at
///   least 4 bits per character; English words stay near 3);
/// - 32 or more hex digits, letters and digits both.
///
/// No quota of digits: a random token may hold none.
pub fn looks_secret(word: &str) -> bool {
    let word: String = word.chars().filter(|c| !invisible(*c)).collect();
    let lower = word.to_ascii_lowercase();
    if SECRET_MARKS.iter().any(|m| lower.contains(m)) || lower.starts_with("akia") {
        return true;
    }
    word.split(|c: char| !c.is_ascii_alphanumeric())
        .any(random_run)
}

fn random_run(run: &str) -> bool {
    let n = run.len();
    if n < 16 {
        return false;
    }
    let class = |c: char| {
        if c.is_ascii_digit() {
            0
        } else if c.is_ascii_uppercase() {
            1
        } else {
            2
        }
    };
    let chars: Vec<char> = run.chars().collect();
    let digits = chars.iter().filter(|c| c.is_ascii_digit()).count();
    let letters = n - digits;
    if n >= 20 && letters > 0 && digits >= 3 {
        return true;
    }
    let switches = chars
        .windows(2)
        .filter(|w| class(w[0]) != class(w[1]))
        .count();
    if switches * 10 >= (n - 1) * 3 {
        return true;
    }
    let one_case = digits == 0
        && (run.chars().all(|c| c.is_ascii_lowercase())
            || run.chars().all(|c| c.is_ascii_uppercase()));
    if n >= 24 && one_case && entropy(run) >= 4.0 {
        return true;
    }
    n >= 32 && digits >= 4 && letters >= 4 && run.chars().all(|c| c.is_ascii_hexdigit())
}

/// Shannon entropy in bits per character.
fn entropy(s: &str) -> f64 {
    let mut counts = [0usize; 128];
    let mut n = 0usize;
    for b in s.bytes().filter(u8::is_ascii) {
        counts[b as usize] += 1;
        n += 1;
    }
    if n == 0 {
        return 0.0;
    }
    counts
        .iter()
        .filter(|c| **c > 0)
        .map(|c| {
            let p = *c as f64 / n as f64;
            -p * p.log2()
        })
        .sum()
}

/// Format characters that change how text reads without being seen: bidi
/// overrides and isolates, zero-width spaces and joiners, the BOM, soft
/// hyphens (Unicode Cf).
pub(crate) fn invisible(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}')
}

// ---------------------------------------------------------------------------
// Masking prose
// ---------------------------------------------------------------------------

/// What a masked secret reads as.
pub const MASK: &str = "••••";

/// Prefixes that mark a credential at the start of a piece.
const SECRET_PREFIXES: [&str; 22] = [
    "sk-",
    "sk_",
    "rk_",
    "pk_live",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "glpat-",
    "xoxa-",
    "xoxb-",
    "xoxp-",
    "xoxr-",
    "xoxs-",
    "akia",
    "asia",
    "aiza",
    "npm_",
    "pypi-",
    "hf_",
];

/// Words in a name that say its value is a secret, in English and Spanish.
const NAME_MARKS: [&str; 15] = [
    "token",
    "secret",
    "passw",
    "apikey",
    "api_key",
    "api-key",
    "authoriz",
    "credential",
    "cookie",
    "contrase",
    "clave",
    "credencial",
    "secreto",
    "private_key",
    "privatekey",
];

/// Punctuation a word may be wrapped in, in prose or in Markdown.
fn wrapper(c: char) -> bool {
    matches!(
        c,
        '"' | '\''
            | '`'
            | '('
            | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '<'
            | '>'
            | ','
            | ';'
            | '.'
            | '!'
            | '?'
            | '¡'
            | '¿'
            | '*'
            | '_'
            | '«'
            | '»'
            | '“'
            | '”'
            | '‘'
            | '’'
            | '|'
    )
}

/// Characters that split a word into pieces: a name, a value, a path part.
fn delimiter(c: char) -> bool {
    matches!(
        c,
        '=' | ':'
            | ','
            | ';'
            | '&'
            | '"'
            | '\''
            | '`'
            | '('
            | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '<'
            | '>'
            | '\\'
            | '/'
            | '|'
            | '#'
            | '?'
            | '*'
            | '@'
            | '“'
            | '”'
            | '‘'
            | '’'
            | '«'
            | '»'
            | '：'
            | '＝'
    )
}

/// Characters that end a value: it runs from after `=` or `:` up to one.
fn terminator(c: char) -> bool {
    matches!(
        c,
        '"' | '\''
            | '`'
            | ','
            | ';'
            | '&'
            | ')'
            | ']'
            | '}'
            | '>'
            | '|'
            | '“'
            | '”'
            | '‘'
            | '’'
            | '»'
            | '<'
    )
}

/// Does `name` (a piece before `=` or `:`, a flag, a bare word) name a
/// secret?
fn secret_name(name: &str) -> bool {
    let n = name
        .trim_start_matches('-')
        .trim_matches(|c: char| wrapper(c))
        .to_lowercase();
    if n.is_empty() {
        return false;
    }
    NAME_MARKS.iter().any(|m| n.contains(m))
        || n == "key"
        || n.ends_with("_key")
        || n.ends_with("-key")
        || n.ends_with(".key")
        || n.ends_with("_pat")
        || n.ends_with("pwd")
        || n == "auth"
        || n.starts_with("auth_")
        || n.ends_with("_auth")
        || n == "pass"
        || n.ends_with("_pass")
        || n == "pin"
        || n == "sid"
        || n == "session"
        || n.ends_with("_session")
}

/// Is one piece of a word a credential by its shape alone?
fn secret_piece(piece: &str) -> bool {
    let lower = piece.to_ascii_lowercase();
    if SECRET_PREFIXES
        .iter()
        .any(|p| lower.starts_with(p) && lower.len() >= p.len() + 8)
    {
        return true;
    }
    // A JSON Web Token: three base64url parts, the first `eyJ`.
    if lower.starts_with("eyj") && piece.matches('.').count() == 2 && piece.len() >= 24 {
        return true;
    }
    uuid(piece)
        || piece
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(random_run)
}

/// A value an unknown name was given that does not read as plain data: six
/// or more characters mixing letters and digits, or holding a symbol, or
/// switching case at least three times. `n=5`, `mode=fast`, `q=serde` and
/// `search=Serialize` stay; `x=hunter123` and `k=Sup3r-S3cret!` go.
fn suspicious(value: &str) -> bool {
    let v = value.trim_matches(|c: char| wrapper(c) || c == '=');
    if v.chars().count() < 6 {
        return false;
    }
    let letters = v.chars().any(char::is_alphabetic);
    let digits = v.chars().any(|c| c.is_ascii_digit());
    let symbol = v
        .chars()
        .any(|c| !c.is_alphanumeric() && !matches!(c, '.' | '-' | '_' | '/' | '+'));
    let switches = v
        .chars()
        .collect::<Vec<_>>()
        .windows(2)
        .filter(|w| {
            w[0].is_alphabetic()
                && w[1].is_alphabetic()
                && w[0].is_uppercase() != w[1].is_uppercase()
        })
        .count();
    (letters && digits) || symbol || switches >= 3
}

/// `NAME`: what an assignment's left side looks like.
fn ident(s: &str) -> bool {
    let s = s.trim_start_matches('-');
    s.chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
        && s.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// Where [`mask_secrets`] is between two pieces or two words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Armed {
    /// Nothing announced.
    No,
    /// A secret's name (`password`): `=`, `:`, `|` or "es"/"is" next arms it.
    Name,
    /// The next value is the one a secret name announced.
    Value,
    /// The word after "session:". On a session bus that is usually a
    /// sentence going on, so only what looks like a credential is one.
    AfterSession,
    /// Inside a quoted value that runs over several words: masked up to
    /// the closing quote.
    Quote(char),
    /// `curl -u user:<v>`: the next word is a user and its password.
    UserPass,
}

/// Words that join a secret's name to its value in a sentence or a table,
/// Spanish included: a credential gets pasted in whatever language is at hand.
const CONNECTORS: [&str; 18] = [
    "=", ":", "|", "：", "＝", "es", "era", "fue", "será", "sería", "son", "queda", "is", "was",
    "are", "be", "->", "→",
];

/// Commands whose flags carry a credential: `mysql -p<v>`, `sshpass -p
/// <v>`, `curl -u user:<v>`.
const PASSWORD_P: [&str; 6] = [
    "mysql",
    "mysqldump",
    "mysqladmin",
    "mariadb",
    "sshpass",
    "mysqlsh",
];
const USER_U: [&str; 3] = ["curl", "wget", "http"];

/// The closing quote of an opening one.
fn closing(q: char) -> char {
    match q {
        '“' => '”',
        '‘' => '’',
        '«' => '»',
        q => q,
    }
}

/// Mask every piece of `text` that reads as a credential, and the value
/// after a name that says it is one, wherever the name sits in a word
/// (`/w/.env:4:DATABASE_PASSWORD=…`, `{"user":"bot","password":"…"}`,
/// `Authorization: Bearer …`, `la contraseña es …`, `| password | … |`), and
/// the password of a URL's userinfo. Whitespace and every other word are
/// kept as written: a message is never masked whole. Invisible format
/// characters are dropped first, so they cannot split a token.
pub fn mask_secrets(text: &str) -> String {
    let text: String = text.chars().filter(|c| !invisible(*c)).collect();
    let mut out = String::with_capacity(text.len());
    let mut armed = Armed::No;
    // The command of this line whose flags carry a credential, if any.
    let mut command: Option<String> = None;
    let mut word_start: Option<usize> = None;
    let flush = |word: &str, armed: &mut Armed, command: &mut Option<String>, out: &mut String| {
        let (masked, next) = mask_one(word, *armed, command.as_deref());
        let bare = word
            .trim_matches(|c: char| wrapper(c))
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if PASSWORD_P.contains(&bare.as_str()) || USER_U.contains(&bare.as_str()) {
            *command = Some(bare);
        } else if matches!(word, ";" | "|" | "&&" | "||") {
            *command = None;
        }
        out.push_str(&masked);
        *armed = next;
    };
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = word_start.take() {
                flush(&text[s..i], &mut armed, &mut command, &mut out);
            }
            if c == '\n' {
                command = None;
                // A blank line ends what a name announced.
                if out.ends_with('\n') {
                    armed = Armed::No;
                }
            }
            out.push(c);
        } else if word_start.is_none() {
            word_start = Some(i);
        }
    }
    if let Some(s) = word_start {
        flush(&text[s..], &mut armed, &mut command, &mut out);
    }
    out
}

/// One word in its context: inside a quoted value, after `curl -u`, a
/// credential flag of the line's command, or the word scanner.
fn mask_one(word: &str, armed: Armed, command: Option<&str>) -> (String, Armed) {
    match armed {
        Armed::Quote(q) => {
            let close = closing(q);
            return match word.find(close) {
                Some(at) => {
                    let (masked, next) = mask_word(&word[at..], Armed::No);
                    (format!("{MASK}{masked}"), next)
                }
                None => (MASK.to_string(), Armed::Quote(q)),
            };
        }
        Armed::UserPass => {
            let (lead, core, trail) = split_wrapped(word);
            let masked = match core.find(':') {
                Some(at) => format!("{}{MASK}", &core[..=at]),
                None => MASK.to_string(),
            };
            return (format!("{lead}{masked}{trail}"), Armed::No);
        }
        _ => {}
    }
    if armed == Armed::No
        && let Some(cmd) = command
    {
        if PASSWORD_P.contains(&cmd) {
            if word == "-p" {
                return (word.to_string(), Armed::Value);
            }
            if let Some(rest) = word.strip_prefix("-p")
                && !rest.is_empty()
                && !rest.starts_with('-')
            {
                return (format!("-p{MASK}"), Armed::No);
            }
        }
        if USER_U.contains(&cmd) && matches!(word, "-u" | "--user") {
            return (word.to_string(), Armed::UserPass);
        }
    }
    mask_word(word, armed)
}

/// A word's leading and trailing wrapper punctuation, and its core.
fn split_wrapped(word: &str) -> (&str, &str, &str) {
    let start = word
        .char_indices()
        .find(|(_, c)| !wrapper(*c))
        .map_or(word.len(), |(i, _)| i);
    let end = word
        .char_indices()
        .rev()
        .find(|(_, c)| !wrapper(*c))
        .map_or(start, |(i, c)| i + c.len_utf8());
    (
        &word[..start],
        &word[start..end.max(start)],
        &word[end.max(start)..],
    )
}

/// A URL's userinfo password (`scheme://user:pass@host`), or the whole
/// userinfo when it has no `:` (`https://<token>@github.com`), masked.
fn mask_userinfo(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    let mut rest = word;
    while let Some(at) = rest.find("://") {
        out.push_str(&rest[..at + 3]);
        rest = &rest[at + 3..];
        let end = rest
            .find(|c: char| matches!(c, '/' | '?' | '#') || terminator(c) || c.is_whitespace())
            .unwrap_or(rest.len());
        let authority = &rest[..end];
        if let Some((info, host)) = authority.rsplit_once('@') {
            match info.split_once(':') {
                Some((user, _)) => {
                    out.push_str(user);
                    out.push(':');
                    out.push_str(MASK);
                }
                None => out.push_str(MASK),
            }
            out.push('@');
            out.push_str(host);
        } else {
            out.push_str(authority);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// A Markdown link whose text is one value that does not read as plain
/// words (`[hunter600](https://…/settings)`): the text is masked, the link
/// stays.
fn mask_link_text(word: &str) -> String {
    if let Some(open) = word.find('[')
        && let Some(close) = word[open..].find("](").map(|c| open + c)
    {
        let text = &word[open + 1..close];
        if !text.is_empty() && !text.contains(['[', ']']) && suspicious(text) {
            return format!("{}{MASK}{}", &word[..=open], &word[close..]);
        }
    }
    word.to_string()
}

/// `8-4-4-4-12` hex digits: a UUID, which may be an API key.
fn uuid(piece: &str) -> bool {
    let parts: Vec<&str> = piece.split('-').collect();
    parts.len() == 5
        && parts
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(p, n)| p.len() == n && p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// One word, masked piece by piece, and what it announces for the next.
fn mask_word(word: &str, armed: Armed) -> (String, Armed) {
    let bare = word.trim_matches(|c: char| wrapper(c));
    if armed == Armed::AfterSession {
        let value = suspicious(bare) || secret_piece(bare);
        return mask_word(word, if value { Armed::Value } else { Armed::No });
    }
    // `la clave es: <v>`: the connector may carry the separator.
    let joined = bare.trim_end_matches([':', '=', '：', '＝']).to_lowercase();
    if armed == Armed::Name
        && (CONNECTORS.contains(&word)
            || CONNECTORS.contains(&bare.to_lowercase().as_str())
            || CONNECTORS.contains(&joined.as_str()))
    {
        return (word.to_string(), Armed::Value);
    }
    if armed == Armed::Name && bare.is_empty() {
        return (word.to_string(), Armed::Name);
    }
    // netrc's `password <v>`, `secret key <v>`: a value right after a bare
    // name is masked when it does not read as a plain word.
    if armed == Armed::Name
        && !bare.is_empty()
        && !secret_name(bare)
        && (suspicious(bare) || secret_piece(bare))
    {
        let (lead, _, trail) = split_wrapped(word);
        return (format!("{lead}{MASK}{trail}"), Armed::No);
    }
    let word = mask_link_text(&mask_userinfo(word));
    let chars: Vec<(usize, char)> = word.char_indices().collect();
    let at = |k: usize| chars.get(k).map_or(word.len(), |(i, _)| *i);
    let mut out = String::with_capacity(word.len());
    let mut state = armed;
    // The secret name just read opened an XML tag: `<password>` arms on `>`.
    let mut in_tag = false;
    let mut k = 0;
    while k < chars.len() {
        let c = chars[k].1;
        // A value a name announced: up to the next terminator.
        if state == Armed::Value && !delimiter(c) {
            let mut j = k;
            while j < chars.len() && !terminator(chars[j].1) {
                j += 1;
            }
            // `**<v>**`, `` `<v>` ``, `<v>.`: the closing marks stay.
            let full = &word[at(k)..at(j)];
            let kept = full.trim_end_matches(['*', '_', '`']);
            let kept = if kept.len() > 1 {
                kept.strip_suffix(['.', ','])
                    .filter(|v| !v.is_empty())
                    .unwrap_or(kept)
            } else {
                kept
            };
            let tail = &full[kept.len()..];
            let value = kept;
            // A quote right before the value that does not close in this
            // word: the value runs on to the next words.
            let opened = k
                .checked_sub(1)
                .map(|p| chars[p].1)
                .filter(|q| matches!(q, '"' | '\'' | '“' | '‘' | '«'));
            let open_quote =
                opened.filter(|q| j == chars.len() && !word[at(k)..].contains(closing(*q)));
            let lower = value.to_ascii_lowercase();
            if matches!(lower.as_str(), "bearer" | "basic" | "token") {
                // `Authorization: Bearer <v>`: the scheme stays.
                out.push_str(value);
            } else if ident(value)
                && secret_name(value)
                && !secret_piece(value)
                && !suspicious(value)
            {
                // `key → --password <v>`: another name, not the value.
                out.push_str(value);
                state = if value.starts_with('-') && j == chars.len() {
                    Armed::Value
                } else {
                    Armed::Name
                };
            } else if let Some(eq) = value.find('=')
                && ident(&value[..eq])
            {
                // `Cookie: session=<v>`: the inner name stays.
                out.push_str(&value[..=eq]);
                if eq + 1 < value.len() {
                    out.push_str(MASK);
                    state = Armed::No;
                }
            } else if value.starts_with("••••") {
                out.push_str(value);
                state = Armed::No;
            } else {
                out.push_str(MASK);
                state = open_quote.map_or(Armed::No, Armed::Quote);
            }
            out.push_str(tail);
            k = j;
            continue;
        }
        if delimiter(c) {
            out.push(c);
            if state == Armed::Name {
                if matches!(c, '=' | ':' | '|' | '：' | '＝') || (c == '>' && in_tag) {
                    state = Armed::Value;
                } else if !matches!(c, '"' | '\'' | '`' | '“' | '”' | '‘' | '’') {
                    state = Armed::No;
                }
            }
            k += 1;
            continue;
        }
        // A piece: up to the next delimiter.
        let mut j = k;
        while j < chars.len() && !delimiter(chars[j].1) {
            j += 1;
        }
        let piece = &word[at(k)..at(j)];
        let next = chars.get(j).map(|(_, c)| *c);
        if secret_piece(piece) {
            out.push_str(MASK);
            state = Armed::No;
        } else if word[..at(k)].ends_with("</") {
            // A closing tag names nothing new.
            out.push_str(piece);
            state = Armed::No;
        } else if secret_name(piece) {
            out.push_str(piece);
            in_tag = word[..at(k)].ends_with('<');
            // `--password <v>`: a flag that names a secret takes the next word.
            state = if piece.starts_with('-') && j == chars.len() {
                Armed::Value
            } else {
                Armed::Name
            };
        } else {
            out.push_str(piece);
            state = Armed::No;
            // `x=<v>` with an unknown name: masked when the value does not
            // read as plain data.
            if matches!(next, Some('=' | '＝')) && ident(piece) {
                let rest = &word[at(j + 1)..];
                let end = rest.find(|c: char| terminator(c)).unwrap_or(rest.len());
                if suspicious(&rest[..end]) {
                    state = Armed::Name;
                }
            }
        }
        k = j;
    }
    if state == Armed::Value
        && bare
            .trim_end_matches([':', '：'])
            .eq_ignore_ascii_case("session")
    {
        state = Armed::AfterSession;
    }
    (out, state)
}

/// The generators of the property tests, shared with the callers' tests
/// (`crate::live`), so the same cases run over each surface.
#[cfg(test)]
pub(crate) mod testkit {
    // ---------------------------------------------------------------------
    // The redaction property over the live view's prose
    // ---------------------------------------------------------------------

    /// xorshift64*: a fixed seed, so a failure names its case.
    pub(crate) struct Rng(pub(crate) u64);
    impl Rng {
        pub(crate) fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        pub(crate) fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        pub(crate) fn pick<'a>(&mut self, xs: &'a [&'a str]) -> &'a str {
            xs[self.below(xs.len())]
        }
        pub(crate) fn chars_of(&mut self, alphabet: &[u8], n: usize) -> String {
            self.chars(alphabet, n)
        }
        pub(crate) fn chars(&mut self, alphabet: &[u8], n: usize) -> String {
            (0..n)
                .map(|_| alphabet[self.below(alphabet.len())] as char)
                .collect()
        }
    }

    pub(crate) const B62: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    pub(crate) const HEX: &[u8] = b"0123456789abcdef";
    pub(crate) const UPPER_DIGITS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    pub(crate) const WORDS: &[&str] = &[
        "the",
        "build",
        "passed",
        "on",
        "main",
        "and",
        "I",
        "will",
        "rebase",
        "onto",
        "it",
        "el",
        "informe",
        "está",
        "listo",
        "revisa",
        "la",
        "rama",
        "directo",
        "luego",
        "commit",
        "tests",
        "fallan",
        "porque",
        "falta",
        "una",
        "migración",
        "token",
        "password",
        "key",
        "secret",
        "Authorization",
        "cargo",
        "bun",
        "README.md",
        "src/live.rs",
        "PR",
        "#42",
        "2026-11-07",
        "10:30",
        "a1b2c3d",
        "ñandú",
        "üñí",
        "→",
        "(ver",
        "abajo)",
        "ok.",
        "listo:",
        "`cargo test`",
    ];

    /// A credential the way it shows up in a reply, and the bare value
    /// that must never come out.
    pub(crate) fn secret(r: &mut Rng) -> (String, String) {
        let v = match r.below(9) {
            0 => format!("ghp_{}", r.chars(B62, 36)),
            1 => format!("github_pat_{}", r.chars(B62, 40)),
            2 => format!("sk-ant-api03-{}", r.chars(B62, 40)),
            3 => format!("AKIA{}", r.chars(UPPER_DIGITS, 16)),
            4 => format!("xoxb-{}-{}", r.chars(b"0123456789", 12), r.chars(B62, 24)),
            5 => r.chars(HEX, 40),
            6 => format!(
                "eyJ{}.{}.{}",
                r.chars(B62, 20),
                r.chars(B62, 30),
                r.chars(B62, 20)
            ),
            7 => {
                // A random run with at least three digits.
                let mut s = r.chars(B62, 22);
                s.push_str(&r.chars(b"0123456789", 3));
                s
            }
            _ => format!("glpat-{}", r.chars(B62, 20)),
        };
        // A weak value is only a secret by what names it.
        let weak = format!("hunter{}", r.below(100));
        let (shown, value) = match r.below(12) {
            0 => (v.clone(), v),
            1 => (format!("`{v}`"), v),
            2 => (format!("({v})."), v),
            3 => (format!("TOKEN={v}"), v),
            4 => (format!("export GITHUB_TOKEN=\"{v}\""), v),
            5 => (format!("Authorization: Bearer {v}"), v),
            6 => (
                format!("https://api.example.com/v1/x?access_token={v}&page=2"),
                v,
            ),
            7 => (format!("https://user:{weak}@example.com/repo.git"), weak),
            8 => (format!("password={weak}"), weak),
            9 => (format!("--password {weak}"), weak),
            10 => (format!("\"api_key\": \"{weak}\","), weak),
            _ => {
                // Zero-width characters cannot split a token past the mask.
                let mid = v.len() / 2;
                (format!("{}\u{200B}{}", &v[..mid], &v[mid..]), v)
            }
        };
        (shown, value)
    }

    // ---------------------------------------------------------------------
    // The review's matrix (harbard-directo-review.md, H1, H2, M1, M2, L1):
    // secrets glued to paths, links, compact JSON, backslash paths, Spanish
    // names, table cells and URL fragments.
    // ---------------------------------------------------------------------

    pub(crate) const B62R: &[u8] = B62;
    pub(crate) const HEXR: &[u8] = HEX;

    /// (class, value). Strong ones have a known shape; weak ones are only a
    /// secret by the name next to them.
    pub(crate) fn review_secret(r: &mut Rng) -> (&'static str, String) {
        match r.below(10) {
            0 => ("ghp", format!("ghp_{}", r.chars_of(B62R, 36))),
            1 => ("sk-ant", format!("sk-ant-api03-{}", r.chars_of(B62R, 40))),
            2 => ("sk_live", format!("sk_live_{}", r.chars_of(B62R, 24))),
            3 => (
                "xoxb",
                format!(
                    "xoxb-{}-{}",
                    r.chars_of(b"0123456789", 12),
                    r.chars_of(B62R, 24)
                ),
            ),
            4 => ("hex40", r.chars_of(HEXR, 40)),
            5 => (
                "uuid",
                format!(
                    "{}-{}-{}-{}-{}",
                    r.chars_of(HEXR, 8),
                    r.chars_of(HEXR, 4),
                    r.chars_of(HEXR, 4),
                    r.chars_of(HEXR, 4),
                    r.chars_of(HEXR, 12)
                ),
            ),
            6 => ("weak", format!("hunter{}", r.below(1000))),
            7 => ("weak-punct", format!("Sup3r-S3cret!{}", r.below(100))),
            8 => ("b64-short", r.chars_of(B62R, 14)),
            _ => (
                "jwt",
                format!(
                    "eyJ{}.{}.{}",
                    r.chars_of(B62R, 20),
                    r.chars_of(B62R, 30),
                    r.chars_of(B62R, 20)
                ),
            ),
        }
    }

    /// Context templates: `{}` is the value.
    pub(crate) const REVIEW_TEMPLATES: &[(&str, &str)] = &[
        ("grep-env", "/x/.env:1:KEY={}"),
        ("grep-env-pass", "/w/app/.env:12:DATABASE_PASSWORD={}"),
        ("grep-env-quoted", "/w/.env.local:3:STRIPE_KEY=\"{}\""),
        (
            "grep-env-nl",
            "encontré esto:\n/w/.env:4:API_KEY={}\nya está",
        ),
        ("query-bare", "?k={}"),
        ("query-token", "?token={}"),
        ("paren-path", "(/tmp/x={})"),
        ("paren-path-named", "(/tmp/x/TOKEN={})"),
        ("json-compact", "{\"user\":\"bot\",\"password\":\"{}\"}"),
        (
            "json-compact-url",
            "{\"api_key\":\"{}\",\"base_url\":\"https://api.example.com\"}",
        ),
        ("json-spaced", "{\"user\": \"bot\", \"password\": \"{}\"}"),
        ("json-tab", "\"password\":\t\"{}\""),
        ("nbsp-name", "password:\u{a0}{}"),
        ("nbsp-eq", "TOKEN=\u{a0}{}"),
        ("tab-eq", "password\t=\t{}"),
        ("auth-glued", "Authorization:Bearer {}"),
        ("auth-json", "{\"Authorization\":\"Bearer {}\"}"),
        ("auth-header", "-H 'Authorization: Bearer {}'"),
        ("userinfo", "https://oauth2:{}@gitlab.com/x.git"),
        ("userinfo-nouser", "https://{}@github.com/o/r.git"),
        ("userinfo-angle", "<https://u:{}@h.io/>"),
        ("url-prefix-glued", "TOKEN={};see=https://x.io/a"),
        ("md-link-text", "[{}](https://x.io/settings)"),
        ("fragment", "https://app.io/cb#access_token={}&state=1"),
        ("dsn", "postgres://app:{}@db:5432/x"),
        ("dsn-named", "DATABASE_URL=postgres://app:{}@db/x"),
        ("es-contrasena", "contraseña: {}"),
        ("es-clave", "clave={}"),
        ("es-prose", "la contraseña es {}"),
        ("flag-eq", "--password={}"),
        ("flag-space", "--api-key {}"),
        ("export-single", "export GH_TOKEN='{}'"),
        ("yaml", "  password: {}"),
        ("table", "| password | {} |"),
        ("dotted", "config.db.password={}"),
        ("pgpass", "PGPASSWORD={} psql"),
        ("cookie", "Cookie: session={}"),
        ("set-cookie-glued", "Set-Cookie:sid={};Path=/"),
        ("aws-secret", "aws_secret_access_key = {}"),
        ("backslash", "C:\\creds\\token={}"),
        ("quoted-path", "'/x/.env:1:SECRET={}'"),
        // The re-review's fresh shapes (harbard-directo-review.md, re-check
        // of ba532df).
        ("es-clave-colon", "la clave es: {}"),
        ("es-token-es", "el token es {}"),
        ("es-contrasena-era", "la contraseña era {}"),
        ("xml-password", "<password>{}</password>"),
        ("xml-apikey", "<apiKey>{}</apiKey> y <user>bot</user>"),
        ("curl-u", concat!("curl -", "u admin:{} https://x.io/a")),
        ("mysql-p-glued", "mysql -u root -p{} app"),
        ("sshpass-p", "sshpass -p '{}' ssh host"),
        ("flag-password-space", "mysql --password {} app"),
        ("netrc", "machine github.com login bot password {}"),
        ("secret-key-space", "secret key {}"),
        ("fullwidth-colon", "contraseña：{}"),
        ("fullwidth-eq", "PASSWORD＝{}"),
        ("quoted-space", "password=\"{} X9z-tail\""),
        ("bold", "La contraseña: **{}** está"),
    ];
}

#[cfg(test)]
mod tests {
    use super::testkit::{REVIEW_TEMPLATES, Rng, WORDS, review_secret, secret};
    use super::*;

    #[test]
    fn property_no_secret_in_masked_prose_and_never_the_whole_message() {
        let mut r = Rng(0x9E37_79B9_7F4A_7C15);
        for case in 0..3_000 {
            let mut text = String::new();
            let mut values = Vec::new();
            let mut plain_words = Vec::new();
            let n = 4 + r.below(30);
            for i in 0..n {
                if i > 0 {
                    text.push_str(r.pick(&[" ", " ", " ", "\n", "\n\n", "  ", "\t"]));
                }
                if r.below(5) == 0 {
                    let (shown, value) = secret(&mut r);
                    text.push_str(&shown);
                    values.push(value);
                    // The word after a secret's name may be taken as its value.
                    text.push(' ');
                    text.push('y');
                } else {
                    let w = r.pick(WORDS);
                    text.push_str(w);
                    plain_words.push(w);
                }
            }
            let shown = &mask_secrets(&text);
            let wire = serde_json::to_string(shown).unwrap();
            for v in &values {
                assert!(
                    !wire.contains(v.as_str()),
                    "case {case}: {v} leaked from {text:?} as {shown:?}"
                );
                // Nor the first or last 16 characters of a long one.
                if v.len() >= 24 {
                    assert!(
                        !shown.contains(&v[..16]),
                        "case {case}: head of {v} in {shown:?}"
                    );
                    assert!(
                        !shown.contains(&v[v.len() - 16..]),
                        "case {case}: tail of {v} in {shown:?}"
                    );
                }
            }
            // Ordinary words survive: the message is never masked whole. A
            // word right after a secret's name may be taken for its value.
            let kept = plain_words.iter().filter(|w| shown.contains(*w)).count();
            let lost = plain_words.len() - kept;
            assert!(
                lost <= 1 + plain_words.len() / 5,
                "case {case}: only {kept} of {} plain words kept in {shown:?}",
                plain_words.len()
            );
            assert!(!shown.trim().is_empty() || text.trim().is_empty());
        }
    }

    /// On a session bus "session:" is mostly prose; what follows is only a
    /// value when it looks like one.
    #[test]
    fn the_word_session_before_a_colon_does_not_hide_the_next_word() {
        let relayed = "Operator check relayed through the directing session: they wrote to you.";
        assert_eq!(mask_secrets(relayed), relayed);
        assert_eq!(
            mask_secrets("Session: main sigue viva"),
            "Session: main sigue viva"
        );
        assert_eq!(
            mask_secrets("session: s3cr3tV4lue99 y sigo"),
            "session: •••• y sigo"
        );
        assert_eq!(
            mask_secrets("Cookie: session=abc123def456"),
            "Cookie: session=••••"
        );
        assert_eq!(mask_secrets("session:abc123def456"), "session:••••");
    }

    #[test]
    fn masking_keeps_prose_readable() {
        let text = "La rama `directo` está lista en a1b2c3d; el token vive en el llavero.\nVer https://docs.rs/serde/latest/serde/ y README.md.";
        assert_eq!(mask_secrets(text), text);
        assert_eq!(
            mask_secrets(&format!("Usa GH_TOKEN=ghp_{} y sigue.", "x".repeat(36))),
            "Usa GH_TOKEN=•••• y sigue."
        );
        assert_eq!(
            mask_secrets("curl -H 'Authorization: Bearer abc.def' https://x.io/a?key=1&page=2"),
            "curl -H 'Authorization: Bearer ••••' https://x.io/a?key=••••&page=2"
        );
        assert_eq!(
            mask_secrets("password = hunter2, luego nada."),
            "password = ••••, luego nada."
        );
    }

    #[test]
    fn the_reviews_matrix_of_shapes_never_leaks_a_secret() {
        let mut r = Rng(0xD1EC_7000_0BAD_F00D);
        let mut leaks = Vec::new();
        for _ in 0..40 {
            for (name, t) in REVIEW_TEMPLATES {
                let (class, v) = review_secret(&mut r);
                let text = format!("Hecho. {} y sigo con la rama.", t.replace("{}", &v));
                let shown = &mask_secrets(&text);
                let wire = serde_json::to_string(shown).unwrap();
                if wire.contains(&v) || shown.contains(&v) {
                    leaks.push(format!("{name}/{class}: {text:?} -> {shown:?}"));
                }
                assert!(
                    shown.starts_with("Hecho. ") && shown.ends_with(" y sigo con la rama."),
                    "{shown:?}"
                );
            }
        }
        assert!(
            leaks.is_empty(),
            "{} leaks:\n{}",
            leaks.len(),
            leaks.join("\n")
        );
    }

    /// The re-review: a quoted value with spaces is masked whole, a bold
    /// value keeps its closing `**`, and plain words near a name stay.
    #[test]
    fn quoted_values_bold_and_plain_words_around_names() {
        let m = mask_secrets;
        assert_eq!(
            m("password=\"Hunt3r X9z-tail\" y sigo"),
            "password=\"•••• ••••\" y sigo"
        );
        assert_eq!(m("PASS='one two three' fin"), "PASS='•••• •••• ••••' fin");
        assert_eq!(
            m("La contraseña: **Pelota2026** está"),
            "La contraseña: **••••** está"
        );
        assert_eq!(
            m("<password>hunter2x</password> ok"),
            "<password>••••</password> ok"
        );
        assert_eq!(
            m(concat!("curl -", "u admin:hunter2x https://x.io")),
            "curl -u admin:•••• https://x.io"
        );
        assert_eq!(
            m("mysql -u root -phunter2x app"),
            "mysql -u root -p•••• app"
        );
        assert_eq!(m("sshpass -p 'hunter2x' ssh h"), "sshpass -p '••••' ssh h");
        assert_eq!(m("la clave es: hunter2x."), "la clave es: ••••.");
        // Plain words and ordinary flags stay.
        for keep in [
            "the password reset flow is done",
            "cargo test -p harbard-core --lib",
            "ssh -p 22 host",
            "la clave del problema está en team.rs",
            "el token caduca mañana",
            "<user>bot</user> y <b>negrita</b>",
            "curl -s https://x.io/a",
        ] {
            assert_eq!(m(keep), keep);
        }
    }
}
