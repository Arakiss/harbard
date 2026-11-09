//! Files written before the stored vocabulary moved to English must keep
//! loading with their meaning. One test per stored family; each writes the
//! old spelling to disk, reads it through the public loader and proves it
//! comes back in today's spelling.

use harbard_core::device::{Access, DeviceRegistry};
use harbard_core::feedback::Sample;
use harbard_core::hold::Digests;
use harbard_core::pairing::{self, Credentials};
use harbard_core::profile::Profile;
use harbard_core::store::Store;
use harbard_core::theme::{DEFAULT_MODE, Mode};
use harbard_core::workspace::DeskState;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static N: AtomicU64 = AtomicU64::new(0);

/// A throwaway harbard home, removed on drop.
struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "harbard-old-files-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Home(p)
    }
    fn write(&self, name: &str, body: &str) {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn old_desk_files_load_in_todays_spelling_and_save_back_new() {
    let home = Home::new();
    home.write(
        "workspace.json",
        r#"{"theme":"berenjena","mode":"oscuro","motion":"reducir","config":{"pensando_long_min":3,"esperando_sleepy_min":4,"ejecutando_long_min":20}}"#,
    );
    let path = home.0.join("workspace.json");
    let desk = DeskState::load_path(&path);
    assert_eq!(desk.theme.as_deref(), Some("aubergine"));
    assert_eq!(desk.mode.as_deref(), Some("dark"));
    assert_eq!(desk.motion.as_deref(), Some("reduce"));
    assert_eq!(desk.config.thinking_long_min, 3);
    assert_eq!(desk.config.waiting_sleepy_min, 4);
    assert_eq!(desk.config.running_long_min, 20);
    // Written back in the new spelling.
    desk.save_path(&path).unwrap();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.contains("\"aubergine\"") && saved.contains("\"dark\""),
        "{saved}"
    );
    assert!(saved.contains("thinking_long_min"), "{saved}");
    for old in ["berenjena", "oscuro", "reducir", "pensando_long_min"] {
        assert!(!saved.contains(old), "{old} in {saved}");
    }
}

#[test]
fn every_old_theme_mode_and_id_keeps_its_meaning() {
    for (old, new) in [
        ("claro", Mode::Light),
        ("oscuro", Mode::Dark),
        ("sistema", Mode::System),
    ] {
        assert_eq!(Mode::parse(old), Some(new));
    }
    assert_eq!(Mode::parse("noche"), None);
    assert_eq!(DEFAULT_MODE, Mode::Light);
    let home = Home::new();
    home.write("workspace.json", r#"{"theme":"grafito","mode":"sistema"}"#);
    let desk = DeskState::load_path(&home.0.join("workspace.json"));
    assert_eq!(desk.theme.as_deref(), Some("graphite"));
    assert_eq!(desk.mode.as_deref(), Some("system"));
}

#[test]
fn an_old_profile_keeps_its_state_and_dnd_default() {
    let home = Home::new();
    home.write(
        "profile.json",
        r#"{"state":{"id":"concentrado","since_ms":1,"until_ms":99},"dnd_default":"manana"}"#,
    );
    let p = Profile::load(&home.0);
    let state = p.state.as_ref().unwrap();
    assert_eq!(state.id, "focused");
    assert_eq!(p.dnd_default.as_deref(), Some("tomorrow"));
}

#[test]
fn old_digests_keep_their_state_and_reason() {
    let home = Home::new();
    home.write(
        "digests.json",
        r#"{"items":[{"id":"d1","state":"fuera","reason":"revisando","from_ms":1,"to_ms":2,"items":[],"read":false}],"cut_ms":2}"#,
    );
    let d = Digests::load(&home.0);
    assert_eq!(d.items[0].state, "away");
    assert_eq!(d.items[0].reason, "reviewing");
    assert_eq!(d.cut_ms, 2);
}

#[test]
fn old_devices_and_credentials_keep_their_roles() {
    let home = Home::new();
    home.write(
        "devices.json",
        r#"{"devices":[
          {"id":"a","name":"A","host":"a.local","role":"satellite","joined_ms":1,"last_seen_ms":1,"access":"operador"},
          {"id":"b","name":"B","host":"b.local","role":"satellite","joined_ms":1,"last_seen_ms":1,"access":"taller"},
          {"id":"c","name":"C","host":"c.local","role":"satellite","joined_ms":1,"last_seen_ms":1,"access":"pantalla"}
        ]}"#,
    );
    home.write(
        "credentials.json",
        r#"{"v":1,"entries":[{"hash":"x","access":"taller","created_ms":1}]}"#,
    );
    let store = Store::at(&home.0);
    let reg = DeviceRegistry::load(&store);
    let access: Vec<Access> = reg.devices.iter().map(|d| d.access).collect();
    assert_eq!(
        access,
        [Access::Operator, Access::Workshop, Access::Display]
    );
    // Written back in the new spelling.
    reg.save(&store).unwrap();
    let saved = std::fs::read_to_string(home.0.join("devices.json")).unwrap();
    assert!(
        saved.contains("\"workshop\"") && saved.contains("\"display\""),
        "{saved}"
    );
    assert!(!saved.contains("taller"), "{saved}");
    assert_eq!(
        Credentials::load(&store).entries[0].access,
        Access::Workshop
    );
    for (old, new) in [
        ("operador", Access::Operator),
        ("taller", Access::Workshop),
        ("pantalla", Access::Display),
    ] {
        assert_eq!(Access::parse(old), Some(new));
        assert_eq!(Access::parse(new.as_str()), Some(new));
    }
}

#[test]
fn an_old_device_log_stays_readable_beside_the_new_one() {
    let home = Home::new();
    let store = Store::at(&home.0);
    home.write(
        "equipos/d1.jsonl",
        r#"{"ts_ms":1,"method":"GET","path":"/old","status":200,"write":false}"#,
    );
    pairing::record(&store, "d1", &pairing::Action::new(2, "POST", "/new", 200)).unwrap();
    let actions = pairing::actions(&store, "d1", 10);
    assert_eq!(
        actions.iter().map(|a| a.path.as_str()).collect::<Vec<_>>(),
        ["/new", "/old"],
        "newest first, the old folder included"
    );
}

#[test]
fn an_old_example_theme_file_is_adopted_under_its_new_name() {
    let home = Home::new();
    home.write("_ejemplo.toml", "# edited by hand\n");
    harbard_core::theme::file::ensure_dir(&home.0).unwrap();
    let adopted = std::fs::read_to_string(home.0.join("_example.toml")).unwrap();
    assert_eq!(adopted, "# edited by hand\n", "the operator's copy is kept");
    assert!(!home.0.join("_ejemplo.toml").exists());
}

#[test]
fn an_old_feedback_sample_keeps_its_activity_state() {
    let s: Sample = serde_json::from_str(r#"{"state":"pensando","label":"cargo test"}"#).unwrap();
    assert_eq!(s.state, "thinking");
    let other: Sample = serde_json::from_str(r#"{"state":"web"}"#).unwrap();
    assert_eq!(other.state, "web");
}
