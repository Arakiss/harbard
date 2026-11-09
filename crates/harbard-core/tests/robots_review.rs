//! The robots review's probes (harbard-robots-review.md, 8149e5c), kept as
//! regression tests on the public `activity` API. Each one failed on that
//! commit. All values are fake.

use harbard_core::activity::{ToolKind, basename, classify, command_label, host, pattern};
use serde_json::json;

fn label(cmd: &str) -> String {
    command_label(cmd).text.unwrap_or_default()
}

fn assert_hidden(cmd: &str, secrets: &[&str]) {
    let l = label(cmd);
    for s in secrets {
        assert!(!l.contains(s), "label {l:?} of {cmd:?} leaks {s:?}");
    }
}

// H1: an env assignment whose quoted value has spaces.

#[test]
fn h1_a_quoted_env_value_with_spaces_never_reaches_the_label() {
    let cmd = r#"DB_PASSWORD="correct horse battery staple" psql -h db.internal"#;
    assert_hidden(cmd, &["horse", "battery", "correct", "staple"]);
    assert_eq!(label(cmd), "psql");
}

#[test]
fn h1_a_quoted_env_value_after_env_never_reaches_the_label() {
    let cmd = r#"env PGPASSWORD='open sesame now' pg_dump prod"#;
    assert_hidden(cmd, &["sesame", "open"]);
    assert_eq!(label(cmd), "pg_dump");
}

#[test]
fn h1_a_quoted_env_value_inside_bash_c_never_reaches_the_label() {
    assert_hidden(
        r#"bash -c "API_KEY='abc hunter two' ./deploy.sh""#,
        &["hunter", "abc"],
    );
}

// H1: quoted text split on ; | & or a newline.

#[test]
fn h1_quoted_text_after_a_separator_is_never_the_program() {
    let cmd = r#"echo "note; hunter2 is the pin""#;
    assert_hidden(cmd, &["hunter2"]);
    assert_eq!(label(cmd), "echo");
}

#[test]
fn h1_multi_line_quoted_text_is_never_the_program() {
    assert_hidden(
        "printf '%s\\n' \"header\nopensesame pin\" > /tmp/x",
        &["opensesame"],
    );
}

#[test]
fn h1_a_url_query_after_an_ampersand_in_quotes_is_never_a_command() {
    let cmd = r#"echo "https://x.test/cb?a=1&opensesame" | pbcopy"#;
    assert_hidden(cmd, &["opensesame"]);
    assert_eq!(label(cmd), "pbcopy");
}

#[test]
fn h1_a_quoted_token_is_never_a_program_or_a_subcommand() {
    assert_eq!(label(r#""ls" -la"#), "");
    assert_eq!(label(r#"git "status""#), "git");
    assert_eq!(label(r#"git 'log' --oneline"#), "git");
}

// M1: tests named only inside quoted text.

#[test]
fn m1_a_commit_message_mentioning_cargo_test_is_not_testing() {
    let l = command_label(r#"git commit -m "fix flaky case; cargo test passes now""#);
    assert!(!l.test, "a git commit reads as a test run: {:?}", l.text);
    assert_eq!(l.text.as_deref(), Some("git commit"));
}

#[test]
fn m1_a_pr_body_listing_the_test_plan_is_not_testing() {
    let l =
        command_label("gh pr create --title x --body \"Summary\n\ncargo test -p harbard-core\n\"");
    assert!(!l.test, "gh pr create reads as a test run: {:?}", l.text);
    assert_eq!(l.text.as_deref(), Some("gh pr"));
}

#[test]
fn m1_a_real_test_run_after_cd_still_is_testing() {
    let l = command_label("cd /w && cargo test --workspace 2>&1 | tail -5");
    assert!(l.test);
    assert_eq!(l.text.as_deref(), Some("cargo test"));
    assert!(command_label(r#"bash -c "cargo test -p x""#).test);
}

// L1: token shapes the digit quota missed, in Grep and in basenames.

#[test]
fn l1_a_base62_token_with_few_digits_is_hidden() {
    let tok = "QmFzZTYyVG9rZW5XaXRoRmVgRGlnaXRz";
    assert!(tok.chars().filter(|c| c.is_ascii_digit()).count() < 3);
    assert_eq!(pattern(tok), None);
}

#[test]
fn l1_a_long_letters_only_key_is_not_shown_truncated() {
    let tok = "kqzvbnwplrtyuxmcoaeihsdgfjkqzvbnwplrtyuxmcoaeihsdgfjkqzvbnwplrty";
    assert_eq!(pattern(tok), None);
}

#[test]
fn l1_a_token_named_file_is_hidden_but_ordinary_names_are_kept() {
    assert_eq!(basename("/tmp/QmFzZTYyVG9rZW5XaXRoRmVgRGlnaXRz"), None);
    for name in [
        "workspace_route.rs",
        "harbard-robots-report.md",
        "2026-11-07-review.md",
        "workspace.css",
        "CHANGELOG.md",
        "theme-context.test.ts",
    ] {
        assert_eq!(basename(&format!("/w/{name}")).as_deref(), Some(name));
    }
    for p in ["fn state_of", "TODO", "harbard_core::activity", "README"] {
        assert_eq!(pattern(p).as_deref(), Some(p));
    }
}

// L2: bidi and zero-width characters.

#[test]
fn l2_bidi_and_zero_width_characters_never_reach_a_label() {
    let d = classify(
        "Read",
        &json!({"file_path": "/w/invoice\u{202E}fdp.exe"}),
        1,
    )
    .detail
    .unwrap_or_default();
    assert!(!d.contains('\u{202E}'), "RLO reaches the label: {d:?}");
    assert_eq!(
        classify("Read", &json!({"file_path": "/w/.to\u{200B}ken"}), 1).detail,
        None
    );
    let g = pattern("fn\u{200D} x\u{2066}y").unwrap_or_default();
    assert!(g.chars().all(|c| !matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')), "{g:?}");
}

// L3: the host a WHATWG parser would fetch, or none.

#[test]
fn l3_a_backslash_or_odd_userinfo_never_names_another_host() {
    assert_ne!(
        host(r"https://evil.test\@docs.rs/").as_deref(),
        Some("docs.rs")
    );
    assert_ne!(
        host("https://docs.rs/x@evil.test").as_deref(),
        Some("evil.test")
    );
    assert_eq!(
        host("https://user:pw@docs.rs:443/x?q=1").as_deref(),
        Some("docs.rs")
    );
    assert_eq!(host("https://DOCS.rs./a").as_deref(), Some("docs.rs"));
    assert_eq!(host("not a url"), None);
}

#[test]
fn sanity_the_builders_cases_still_hold() {
    assert_eq!(
        label("cd /x && CARGO_TARGET_DIR=/t cargo test -p y 2>&1 | tail"),
        "cargo test"
    );
    assert_eq!(
        classify("Bash", &json!({"command":"cargo test"}), 1).kind,
        ToolKind::Test
    );
    assert_eq!(label("bun run build"), "bun run build");
    assert_eq!(label("git -C /repo status"), "git status");
}

// ---------------------------------------------------------------------------
// Round 2 (harbard-robots-review.md, "Round 2: fcf628c"): the reviewer's
// probe2 attacks, kept as regression tests.
// ---------------------------------------------------------------------------

#[test]
fn r2_n1_ansi_c_quotes_keep_an_escaped_quote_inside_one_word() {
    let cmd = r"PGPASSWORD=$'it\'s hunter two now' psql -h db";
    assert_hidden(cmd, &["hunter", "two"]);
    assert_eq!(label(cmd), "psql");
    assert_hidden(r"echo $'don\'t; opensesame here'", &["opensesame"]);
    assert_hidden("bash -c $'echo don\\'t; hunter2 x'", &["hunter2"]);
}

#[test]
fn r2_n2_an_array_assignment_is_one_word_and_never_a_command() {
    assert_hidden(
        "PASSWORDS=(opensesame hunter) ; cargo build",
        &["opensesame", "hunter"],
    );
    assert_eq!(
        label("PASSWORDS=(opensesame hunter) ; cargo build"),
        "cargo build"
    );
    assert_eq!(
        label("ARGS=(--release -p core); cargo build \"${ARGS[@]}\""),
        "cargo build"
    );
}

#[test]
fn r2_n3_a_plumbing_line_is_labelled_by_its_program_alone() {
    assert_eq!(label("echo opensesame"), "echo");
    assert_eq!(label("printf opensesame"), "printf");
    assert_eq!(label("cat notes"), "cat");
    assert_hidden("PW=$(echo opensesame)", &["opensesame"]);
    assert_hidden("PW=`echo opensesame`", &["opensesame"]);
    assert_hidden("NOTE=`cat notes` git status", &["notes"]);
}

#[test]
fn r2_n4_a_backslash_newline_never_starts_an_empty_word() {
    let l = command_label("CARGO_TARGET_DIR=/t \\\n  cargo test -p harbard-core");
    assert_eq!((l.text.as_deref(), l.test), (Some("cargo test"), true));
    let l = command_label("cd /x && \\\n  bun test src");
    assert_eq!((l.text.as_deref(), l.test), (Some("bun test"), true));
}

#[test]
fn r2_n5_reserved_words_are_skipped_and_a_loop_variable_never_shows() {
    let l = command_label("if grep -q x f; then cargo test; fi");
    assert_eq!((l.text.as_deref(), l.test), (Some("cargo test"), true));
    // The loop variable never shows, nor rg's pattern.
    assert_eq!(label("for f in a b; do rg x $f; done"), "rg");
    assert_eq!(label("grep -rn secretword src"), "grep");
    assert_hidden("for secretname in a b; do echo x; done", &["secretname"]);
    assert_eq!(label("while true; do bun test; done"), "bun test");
    assert_eq!(label("! git diff --quiet"), "git diff");
    assert_eq!(label("{ cargo build; }"), "cargo build");
    assert_eq!(label("case $x in a) echo; esac"), "echo");
}

#[test]
fn r2_n6_a_redirection_keeps_its_ampersand() {
    assert_eq!(label("ls -la 2>&1"), "ls");
    // All plumbing: the first command names the line; never the `1`.
    assert_eq!(label("cd /x && ls 2>&1 | head"), "cd");
    assert_eq!(label("cargo build &> /tmp/log"), "cargo build");
    assert_eq!(label("cargo build >& /tmp/log"), "cargo build");
    assert_eq!(
        label("cargo build 2> /tmp/err && git status"),
        "cargo build"
    );
}

#[test]
fn r2_the_probe2_attacks_that_passed_still_pass() {
    assert_hidden(
        r#"bash -c "FOO='a b' sh -c 'echo \"x; hunter2 y\"'""#,
        &["hunter2"],
    );
    assert_hidden(r#"bash -c 'X="open sesame now" psql'"#, &["sesame"]);
    assert_hidden(r#"echo "say \"hi; hunter2 there\"""#, &["hunter2"]);
    assert_hidden(r#"mysql -u root <<< "SELECT 1; hunter2 x""#, &["hunter2"]);
    assert_hidden("psql <<<'x | opensesame y'", &["opensesame"]);
    assert_hidden(
        "cat <<EOF | ssh host\nhunter2 is the pin\nEOF",
        &["hunter2"],
    );
    assert_eq!(
        host(r"https://evil.test\@docs.rs/").as_deref(),
        Some("evil.test")
    );
    assert_eq!(pattern("QmFzZTYyVG9rZW5XaXRoRmVgRGlnaXRz"), None);
}

// ---------------------------------------------------------------------------
// Round 3 (harbard-robots-review.md, "Round 3: 20bb8e8"): probe3, and the
// class fix — a program is shown only when it is a known executable, a
// subcommand only for a short list of programs.
// ---------------------------------------------------------------------------

#[test]
fn r3_n1_an_array_scan_honours_backslashes() {
    assert_hidden(
        r"A=(a\) opensesame word) ; cargo build",
        &["opensesame", "word"],
    );
    assert_eq!(
        label(r"A=(a\) opensesame word) ; cargo build"),
        "cargo build"
    );
    assert_hidden(
        r#"A=("x\"y) opensesame word" z) ; cargo build"#,
        &["opensesame"],
    );
    assert_eq!(
        label(r#"A=("x) opensesame" 'b)c') ; cargo build"#),
        "cargo build"
    );
}

#[test]
fn r3_n2_case_patterns_after_a_double_semicolon_are_not_commands() {
    let cmd = r#"case "$x" in a) true ;; opensesame) cargo build ;; esac"#;
    assert_hidden(cmd, &["opensesame"]);
    assert_eq!(label(cmd), "cargo build");
    let multi =
        "case $x in\n  a|b) echo one ;;\n  (zebrapattern) cargo test ;;\n  *) true ;;\nesac";
    assert_hidden(multi, &["zebrapattern"]);
    assert!(command_label(multi).test);
}

#[test]
fn r3_n3_arithmetic_is_part_of_its_word() {
    assert_eq!(label("N=$((1+2)) cargo build"), "cargo build");
    assert_eq!(label("TOTAL=$((end-start)) cargo build"), "cargo build");
    assert_hidden(
        r#"PW=$(printf '%s' "$(op read 'op://v/opensesame word')") psql"#,
        &["opensesame"],
    );
    assert_eq!(label(r"X=$'a\\' cargo build"), "cargo build");
    assert_hidden(r"X=$'\x27 opensesame word' psql", &["opensesame"]);
    assert_hidden(r#"echo "$'" opensesame word "'""#, &["opensesame"]);
}

#[test]
fn r3_an_unknown_program_is_never_named() {
    // Not on the allowlist and (in tests) no PATH: "Running a command".
    assert_eq!(command_label("zzunknowntool --flag").text, None);
    assert_eq!(command_label("./deploy.sh prod").text, None);
    assert_eq!(command_label("/opt/tools/zzfrobnicate run").text, None);
    // A known program by its path is still known.
    assert_eq!(label("/opt/homebrew/bin/rg -n x src"), "rg");
}

#[test]
fn r3_a_subcommand_shows_only_for_listed_programs() {
    assert_eq!(label("ssh myhost uptime"), "ssh");
    assert_eq!(label("PW=$(decrypt opensesame) psql -h db"), "psql");
    assert_eq!(label("docker compose up"), "docker compose");
    assert_eq!(label("cargo test -p x"), "cargo test");
    assert_eq!(label("git status"), "git status");
    assert_eq!(label("bun run build"), "bun run build");
    assert_eq!(label("brew install x"), "brew install");
    assert_eq!(label("npx vitest run"), "npx");
    assert_eq!(label("make release"), "make");
    // A subcommand must look like one.
    assert_eq!(label("git a"), "git");
    assert_eq!(label("git some_thing"), "git");
}

#[test]
fn r3_known_programs_on_path_are_named_and_others_are_not() {
    // serve registers the names it found on PATH once, at startup.
    harbard_core::activity::set_path_programs(["zzpathtool".to_string()]);
    assert_eq!(label("zzpathtool --x"), "zzpathtool");
    assert_eq!(label("zzpathtool sub"), "zzpathtool");
    assert_eq!(command_label("zznotonpath --x").text, None);
}

/// Random secret-looking words (lowercase, 8–14 letters, never an allowlist
/// name) in every argument and assignment position of a corpus of shapes
/// never reach a label.
#[test]
fn r3_property_a_secret_word_in_any_argument_never_reaches_a_label() {
    let shapes = [
        "cargo test {S}",
        "cargo {S} build",
        "git commit -m {S}",
        "git {S}",
        "gh pr create --title {S}",
        "bun run {S}",
        "npm run {S} --x",
        "docker {S} up",
        "{S}=x cargo build",
        "X={S} cargo build",
        "X=\"{S} {S}\" cargo build",
        "X='{S}' psql -h db",
        "env X={S} {S} pg_dump {S}",
        "A=({S} {S}) ; cargo build {S}",
        "A=(a\\) {S}) ; cargo build",
        "PW=$(echo {S}) psql",
        "PW=$({S} {S}) psql",
        "PW=`{S} {S}` git status",
        "N=$(( {S} + 1 )) cargo build",
        "echo {S}",
        "printf '%s' {S}",
        "cat {S}",
        "grep -rn {S} src",
        "rg {S} --glob x",
        "ssh {S} uptime",
        "curl https://{S}.example/x?{S}=1",
        "{S} --flag {S}",
        "./{S} prod",
        "/usr/local/bin/{S} run",
        "bash -c \"{S} {S}\"",
        "sh -c '{S}; {S}'",
        "case $x in {S}) cargo build ;; {S}) true ;; esac",
        "for {S} in a b; do {S} x; done",
        "if {S}; then {S}; fi",
        "cd {S} && {S} {S}",
        "{S} | {S} > {S}",
        "{S} 2>&1 | tail",
        "timeout 30 {S} {S}",
        "sudo {S} {S}",
        "python3 -m {S}",
        "make {S}",
        "just {S}",
        "brew {S}_x",
        "X=$'{S}\\'s' cargo build",
    ];
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for round in 0..400 {
        for shape in shapes {
            let mut words = Vec::new();
            let mut cmd = String::new();
            for (i, part) in shape.split("{S}").enumerate() {
                if i > 0 {
                    let len = 8 + (next() % 7) as usize;
                    let w: String = (0..len)
                        .map(|_| (b'a' + (next() % 26) as u8) as char)
                        .collect();
                    let w = format!("q{w}");
                    cmd.push_str(&w);
                    words.push(w);
                }
                cmd.push_str(part);
            }
            let l = label(&cmd);
            for w in &words {
                assert!(
                    !l.contains(w.as_str()),
                    "round {round}: {w:?} reached the label {l:?} of {cmd:?}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Round 4 (harbard-robots-review.md, "Round 4: 8ac3cb6"): probe4. Words that
// are also on PATH stand in for a secret that happens to be a program name.
// ---------------------------------------------------------------------------

fn on_path_label(cmd: &str) -> String {
    harbard_core::activity::set_path_programs(["open", "say", "opensesame"].map(String::from));
    label(cmd)
}

fn on_path_hidden(cmd: &str, s: &str) {
    let l = on_path_label(cmd);
    assert!(!l.contains(s), "label {l:?} of {cmd:?} leaks {s:?}");
}

#[test]
fn r4_q1_q4_q8_what_held_in_round_four_still_holds() {
    assert_eq!(on_path_label("bun run opensesame"), "bun run");
    assert_eq!(on_path_label("npm run opensesame --x"), "npm run");
    on_path_hidden(
        r#"case "$x" in a|opensesame) cargo build ;; esac"#,
        "opensesame",
    );
    on_path_hidden(
        r#"case "$x" in (opensesame) cargo build ;; esac"#,
        "opensesame",
    );
    on_path_hidden(
        "select v in opensesame say; do cargo build; done",
        "opensesame",
    );
    on_path_hidden("for v in opensesame; do cargo build; done", "opensesame");
    on_path_hidden(r#"echo "x; opensesame y""#, "opensesame");
    on_path_hidden(r"PW=$'a\'b opensesame c' psql", "opensesame");
}

#[test]
fn r4_n2_option_values_of_sudo_and_env_are_never_the_program() {
    for cmd in [
        "sudo -g opensesame cargo build",
        "sudo -u opensesame cargo build",
        "sudo -C 3 -u opensesame cargo build",
        "sudo -h opensesame cargo build",
        "sudo -p opensesame -U say cargo build",
        "env -C opensesame cargo build",
        "env -u opensesame cargo build",
        "env -S opensesame cargo build",
        "env -P opensesame cargo build",
    ] {
        on_path_hidden(cmd, "opensesame");
        assert_eq!(on_path_label(cmd), "cargo build", "{cmd}");
    }
}

#[test]
fn r4_n2_a_function_definition_is_not_a_run() {
    on_path_hidden("opensesame() { :; }; cargo build", "opensesame");
    assert_eq!(
        on_path_label("opensesame() { :; }; cargo build"),
        "cargo build"
    );
    on_path_hidden("function opensesame { :; }; cargo build", "opensesame");
    on_path_hidden("say () { echo x; }; git status", "say");
    assert_eq!(
        on_path_label("say () { echo x; }; git status"),
        "git status"
    );
}
