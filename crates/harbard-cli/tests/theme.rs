//! `harbard theme` and `/api/v1/theme` end to end: the real binary, a temp
//! home, a cleared environment. Nothing reads the operator's own home.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static SEQ: AtomicU64 = AtomicU64::new(0);
const TOKEN: &str = "harbard-theme-test-token";

const MINIMAL: &str = "frame = \"#12324a\"\naccent = \"#2b7bb9\"\n";
const UNKNOWN_KEY: &str =
    "frame = \"#12324a\"\naccent = \"#2b7bb9\"\nsidebar_hover = \"#000000\"\n";
const BAD_HEX: &str = "frame = \"#12324\"\naccent = \"#2b7bb9\"\n";
const LOW_CONTRAST: &str = "frame = \"#3f0e40\"\naccent = \"#ffd000\"\naccent-ink = \"#ffffff\"\n";

struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("harbard-theme-it-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("themes")).unwrap();
        Home(d)
    }
    fn theme(&self, name: &str, text: &str) -> PathBuf {
        let p = self.0.join("themes").join(name);
        std::fs::write(&p, text).unwrap();
        p
    }
    /// No serve unless a test points `HARBARD_API` at one: never the
    /// operator's on 7419.
    fn run(&self, args: &[&str]) -> Output {
        self.run_with_api(args, "http://127.0.0.1:9")
    }
    fn run_with_api(&self, args: &[&str], api: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_harbard"))
            .env_clear()
            .env("HOME", &self.0)
            .env("HARBARD_API", api)
            .arg("--home")
            .arg(&self.0)
            .args(args)
            .output()
            .expect("spawn harbard")
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn theme_check_accepts_a_minimal_file_and_refuses_each_bad_one_with_its_reason() {
    let home = Home::new();
    let ok = home.run(&[
        "theme",
        "check",
        home.theme("mar.toml", MINIMAL).to_str().unwrap(),
    ]);
    assert_eq!(ok.status.code(), Some(0), "{}", err(&ok));
    assert!(
        out(&ok).contains("ok:") && out(&ok).contains("\"mar\""),
        "{}",
        out(&ok)
    );

    for (name, text, expect) in [
        (
            "desconocida.toml",
            UNKNOWN_KEY,
            "`sidebar_hover`: `sidebar_hover` is not a theme key",
        ),
        (
            "hex.toml",
            BAD_HEX,
            "`frame`: `#12324` is not a hexadecimal colour",
        ),
        ("bajo.toml", LOW_CONTRAST, "`accent-ink` on `accent`"),
    ] {
        let path = home.theme(name, text);
        let o = home.run(&["theme", "check", path.to_str().unwrap()]);
        assert_eq!(o.status.code(), Some(1), "{name}");
        let reason = err(&o);
        assert!(reason.contains(name), "names the file: {reason}");
        assert!(reason.contains(expect), "{name}: {reason}");
        assert!(reason.contains("Fix:"), "names the fix: {reason}");
    }
}

/// Installed hooks may still pass the Spanish spellings: they stay as hidden
/// aliases that resolve to the English values.
#[test]
fn the_old_mode_spellings_are_hidden_aliases() {
    let home = Home::new();
    let set = home.run(&["theme", "mode", "oscuro"]);
    assert!(set.status.success(), "{}", err(&set));
    let list = home.run(&["theme", "list"]);
    assert!(out(&list).contains("Mode: Dark (dark)"), "{}", out(&list));
    let help = home.run(&["theme", "mode", "--help"]);
    assert!(!out(&help).contains("oscuro"), "{}", out(&help));
}

#[test]
fn theme_set_mode_and_list_agree_and_refuse_unknown_names() {
    let home = Home::new();
    home.theme("mar.toml", MINIMAL);
    let list = home.run(&["theme", "list"]);
    assert!(out(&list).contains("* ink"), "{}", out(&list));
    assert!(out(&list).contains("Mode: Light (light)"));

    let set = home.run(&["theme", "set", "mar"]);
    assert_eq!(set.status.code(), Some(0), "{}", err(&set));
    assert!(
        out(&set).starts_with("Theme: mar (mar). Saved to "),
        "{}",
        out(&set)
    );
    let mode = home.run(&["theme", "mode", "dark"]);
    assert!(
        out(&mode).starts_with("Mode: Dark (dark)."),
        "{}",
        out(&mode)
    );

    let list = out(&home.run(&["theme", "list"]));
    assert!(list.contains("* mar"), "{list}");
    assert!(list.contains("  ink"), "{list}");
    assert!(list.contains("Mode: Dark"), "{list}");

    let missing = home.run(&["theme", "set", "nada"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(
        err(&missing).contains("there is no theme `nada`"),
        "{}",
        err(&missing)
    );
    // A refused file cannot be chosen, and the previous choice stays.
    home.theme("bajo.toml", LOW_CONTRAST);
    let bad = home.run(&["theme", "set", "bajo"]);
    assert_eq!(bad.status.code(), Some(1));
    assert!(out(&home.run(&["theme", "list"])).contains("* mar"));
}

#[test]
fn theme_import_creates_the_file_and_path_prints_the_folder() {
    let home = Home::new();
    std::fs::remove_dir_all(home.0.join("themes")).unwrap();
    let path = home.run(&["theme", "path"]);
    assert_eq!(
        out(&path).trim(),
        home.0.join("themes").display().to_string()
    );
    assert!(home.0.join("themes/_example.toml").exists());

    home.theme("mar.toml", MINIMAL);
    let list = out(&home.run(&["theme", "list"]));
    assert!(list.contains("mar"));
    // The code of a built-in, renamed, is a new file with the same tokens.
    let server = Serve::start(&home.0);
    let body = server.get("/api/v1/theme");
    let jade = theme_of(&body, "jade");
    let code = jade["share"]
        .as_str()
        .unwrap()
        .replace(":jade:", ":mi-jade:");
    drop(server);
    let imported = home.run(&["theme", "import", &code]);
    assert_eq!(imported.status.code(), Some(0), "{}", err(&imported));
    assert!(out(&imported).contains("Imported:"), "{}", out(&imported));
    let file = home.0.join("themes/mi-jade.toml");
    assert!(file.exists());
    let again = home.run(&["theme", "import", &code]);
    assert_eq!(
        again.status.code(),
        Some(0),
        "the same content again is fine"
    );
    let check = home.run(&["theme", "check", file.to_str().unwrap()]);
    assert_eq!(check.status.code(), Some(0), "{}", err(&check));
    let broken = home.run(&["theme", "import", "harbard-theme:v1:x:#000"]);
    assert_eq!(broken.status.code(), Some(1));
    assert!(err(&broken).contains("share code"), "{}", err(&broken));
}

// ---------------------------------------------------------------------------
// serve
// ---------------------------------------------------------------------------

struct Serve {
    child: Child,
    port: u16,
}

impl Serve {
    fn start(home: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_harbard"))
            .env_clear()
            .env("HOME", home)
            .arg("--home")
            .arg(home)
            .args(["serve", "--port", "0", "--token", TOKEN])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn serve");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        Serve {
            child,
            port: v["port"].as_u64().unwrap() as u16,
        }
    }

    fn send(&self, method: &str, path: &str, body: &str) -> (u16, serde_json::Value) {
        let mut sock = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        sock.write_all(req.as_bytes()).unwrap();
        let mut raw = String::new();
        sock.read_to_string(&mut raw).unwrap();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap();
        let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        // A large answer (every built-in theme, both modes) comes chunked.
        let body = if head
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
        {
            dechunk(body)
        } else {
            body.to_string()
        };
        (
            status,
            serde_json::from_str(&body).unwrap_or(serde_json::Value::Null),
        )
    }

    fn get(&self, path: &str) -> serde_json::Value {
        let (status, body) = self.send("GET", path, "");
        assert_eq!(status, 200, "{body}");
        body
    }

    fn post(&self, action: &str, body: serde_json::Value) -> (u16, serde_json::Value) {
        self.send(
            "POST",
            &format!("/api/v1/workspace/{action}"),
            &body.to_string(),
        )
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// An HTTP/1.1 chunked body, joined. Sizes count bytes, so it walks bytes.
fn dechunk(body: &str) -> String {
    let mut rest = body.as_bytes();
    let mut out = Vec::new();
    while let Some(end) = rest.windows(2).position(|w| w == b"\r\n") {
        let size = std::str::from_utf8(&rest[..end]).unwrap_or("0");
        let len = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        let data = &rest[end + 2..];
        if len == 0 || data.len() < len {
            break;
        }
        out.extend_from_slice(&data[..len]);
        rest = data[len..].strip_prefix(b"\r\n").unwrap_or(&data[len..]);
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn theme_of<'a>(body: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    body["themes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap_or_else(|| panic!("no theme {id} in {body}"))
}

#[test]
fn the_stored_theme_and_mode_survive_a_serve_restart() {
    let home = Home::new();
    let first = Serve::start(&home.0);
    let body = first.get("/api/v1/theme");
    assert_eq!(
        (body["theme"].as_str(), body["mode"].as_str()),
        (Some("ink"), Some("light"))
    );
    assert!(body["stored"]["theme"].is_null(), "nothing stored yet");
    let (status, _) = first.post("theme-set", serde_json::json!({ "theme": "lagoon" }));
    assert_eq!(status, 200);
    let (status, _) = first.post("theme-mode", serde_json::json!({ "mode": "system" }));
    assert_eq!(status, 200);
    drop(first);

    let second = Serve::start(&home.0);
    let body = second.get("/api/v1/theme");
    assert_eq!(body["theme"], "lagoon");
    assert_eq!(body["mode"], "system");
    assert_eq!(body["stored"]["theme"], "lagoon");
    // Both palettes ship, so "Same as system" switches without a request.
    assert!(theme_of(&body, "lagoon")["dark"]["frame"].is_string());
    assert_eq!(body["modes"]["dark"]["color-scheme"], "dark");
}

#[test]
fn an_edited_theme_file_reaches_serve_within_seconds_and_a_broken_edit_keeps_the_last_tokens() {
    let home = Home::new();
    let path = home.theme("mar.toml", MINIMAL);
    let serve = Serve::start(&home.0);
    let (status, _) = serve.post("theme-set", serde_json::json!({ "theme": "mar" }));
    assert_eq!(status, 200);
    let before = serve.get("/api/v1/theme");
    let revision = before["revision"].as_str().unwrap().to_string();
    let unchanged = serve.get(&format!("/api/v1/theme?since={revision}"));
    assert_eq!(unchanged["unchanged"], true);

    std::fs::write(&path, "frame = \"#12324a\"\naccent = \"#b4235a\"\n").unwrap();
    let started = Instant::now();
    let after = loop {
        let body = serve.get(&format!("/api/v1/theme?since={revision}"));
        if body["unchanged"] != true {
            break body;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "no change after 5 s"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(theme_of(&after, "mar")["light"]["accent"], "#b4235a");
    let good = theme_of(&after, "mar")["light"].clone();

    for (text, code) in [
        (UNKNOWN_KEY, "unknown_key"),
        (BAD_HEX, "bad_colour"),
        (LOW_CONTRAST, "low_contrast"),
    ] {
        std::fs::write(&path, text).unwrap();
        let started = Instant::now();
        let body = loop {
            let body = serve.get("/api/v1/theme");
            if body["errors"].as_array().is_some_and(|e| !e.is_empty())
                && body["errors"][0]["code"] == code
            {
                break body;
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "{code} not reported"
            );
            std::thread::sleep(Duration::from_millis(100));
        };
        assert_eq!(body["theme"], "mar", "{code}: still applied");
        let entry = theme_of(&body, "mar");
        assert_eq!(entry["stale"], true);
        assert_eq!(entry["light"], good, "{code}: the previous tokens stay");
        assert!(
            body["errors"][0]["file"]
                .as_str()
                .unwrap()
                .ends_with("mar.toml")
        );
    }
}

#[test]
fn the_picker_actions_refuse_with_the_reason() {
    let home = Home::new();
    let serve = Serve::start(&home.0);
    let (status, body) = serve.post(
        "theme-save",
        serde_json::json!({ "name": "jade", "frame": "#12324a", "accent": "#2b7bb9" }),
    );
    assert_eq!(status, 422);
    assert_eq!(body["refusal"]["code"], "bad_name", "{body}");
    let (status, body) = serve.post(
        "theme-save",
        serde_json::json!({ "name": "mar", "frame": "#12324a", "accent": "#2b7bb9" }),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["theme"], "mar");
    let (status, _) = serve.post(
        "theme-save",
        serde_json::json!({ "name": "mar", "frame": "#12324a", "accent": "#1f6f9f" }),
    );
    assert_eq!(status, 409, "an existing different file is not overwritten");
}

/// Ported from the review (M1, `review_desk_race`): `harbard theme set` is a
/// second process writing `workspace.json` while serve writes it through
/// `DeskState::update` (what `with_desk` runs). Neither may lose the other's
/// write, and the CLI never fails on the shared file.
#[test]
fn cli_theme_writes_never_lose_serve_desk_writes() {
    use harbard_core::workspace::DeskState;
    let home = Home::new();
    let path = DeskState::path(&home.0);
    let cli_home = home.0.clone();
    let cli = std::thread::spawn(move || {
        let mut failures = Vec::new();
        for i in 0..40 {
            let name = if i % 2 == 0 { "jade" } else { "lagoon" };
            let st = Command::new(env!("CARGO_BIN_EXE_harbard"))
                .env_clear()
                .env("HOME", &cli_home)
                // No serve here: the CLI writes the file itself.
                .env("HARBARD_API", "http://127.0.0.1:9")
                .arg("--home")
                .arg(&cli_home)
                .args(["theme", "set", name])
                .output()
                .unwrap();
            if !st.status.success() {
                failures.push(String::from_utf8_lossy(&st.stderr).trim().to_string());
            }
        }
        failures
    });
    let mut n = 0u32;
    while !cli.is_finished() {
        DeskState::update(&path, |desk| {
            desk.threads.insert(format!("note-{n}"), format!("msg-{n}"));
        })
        .unwrap();
        n += 1;
    }
    let failures = cli.join().unwrap();
    let desk = DeskState::load_path(&path);
    assert!(
        failures.is_empty(),
        "`harbard theme set` failed {} times: {:?}",
        failures.len(),
        failures.first()
    );
    assert_eq!(
        desk.threads.len() as u32,
        n,
        "thread mappings lost to concurrent `harbard theme set`"
    );
    assert!(matches!(desk.theme.as_deref(), Some("jade" | "lagoon")));
}

#[test]
fn theme_set_never_overwrites_a_desk_it_cannot_read() {
    let home = Home::new();
    let path = home.0.join("workspace.json");
    let broken = "{ \"config\": { \"operator_name\": \"Dana\" }, \"threads\": ";
    std::fs::write(&path, broken).unwrap();
    let o = home.run(&["theme", "set", "jade"]);
    assert_eq!(o.status.code(), Some(1), "{}", out(&o));
    assert!(err(&o).contains("left untouched"), "{}", err(&o));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
}

#[test]
fn with_serve_up_for_this_home_the_cli_writes_through_it() {
    let home = Home::new();
    let serve = Serve::start(&home.0);
    let api = format!("http://127.0.0.1:{}", serve.port);
    std::fs::write(home.0.join("hub.token"), format!("{TOKEN}\n")).unwrap();
    let o = home.run_with_api(&["theme", "set", "jade"], &api);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(
        out(&o).contains(&format!("Saved to serve ({api})")),
        "{}",
        out(&o)
    );
    assert_eq!(serve.get("/api/v1/theme")["theme"], "jade");
    let o = home.run_with_api(&["theme", "mode", "dark"], &api);
    assert!(out(&o).contains("Saved to serve"), "{}", out(&o));
    assert_eq!(serve.get("/api/v1/theme")["mode"], "dark");

    // A serve for another home is not ours: the CLI writes the file.
    let other = Home::new();
    let o = other.run_with_api(&["theme", "set", "lagoon"], &api);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(out(&o).contains("workspace.json"), "{}", out(&o));
    assert_eq!(
        serve.get("/api/v1/theme")["theme"],
        "jade",
        "the other serve is untouched"
    );
}
