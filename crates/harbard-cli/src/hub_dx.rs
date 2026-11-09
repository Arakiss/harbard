//! Hub / satellite DX helpers (M4a) — files under `~/.harbard/` so pairing is
//! almost plug-and-play instead of a scavenger hunt for env vars.
//!
//! | File | Role |
//! |---|---|
//! | `hub.token` | The hub's own key: its local console and CLI, never a device's |
//! | `credentials.json` | Hashes of each device's own token (hub, 0600) |
//! | `satellite.json` | `{ "hub", "token", "device_id", "name" }` on a device (0600) |
//!
//! Env still wins when set. Files fill the gaps.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use harbard_core::Store;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

pub(crate) fn write_private(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("harbard.tmp");
    // Created 0600 from its first byte: a secret is never readable by others,
    // not even between the write and a later chmod.
    match fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    {
        use std::io::Write;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(contents.as_ref())?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Persisted satellite pairing (written by `harbard pair`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SatelliteConfig {
    pub hub: String,
    pub token: String,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub name: String,
}

impl SatelliteConfig {
    pub fn path(store: &Store) -> PathBuf {
        store.root().join("satellite.json")
    }

    pub fn load(store: &Store) -> Option<Self> {
        let raw = fs::read_to_string(Self::path(store)).ok()?;
        serde_json::from_str(&raw).ok()
    }

    pub fn save(&self, store: &Store) -> io::Result<()> {
        let path = Self::path(store);
        let raw = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::other(format!("satellite serialize: {e}")))?;
        write_private(&path, raw)
    }

    pub fn remove(store: &Store) -> io::Result<()> {
        let path = Self::path(store);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}

pub fn hub_token_path(store: &Store) -> PathBuf {
    store.root().join("hub.token")
}

pub fn read_hub_token(store: &Store) -> Option<String> {
    fs::read_to_string(hub_token_path(store))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn write_hub_token(store: &Store, token: &str) -> io::Result<()> {
    let path = hub_token_path(store);
    write_private(&path, format!("{}\n", token.trim()))
}

/// The hub's per-device credentials (SHA-256 hashes only), when the hub has
/// them. `rotate` never touches this file.
pub fn credentials_path(store: &Store) -> PathBuf {
    store.root().join("credentials.json")
}

/// What `harbard hub rotate` did, for its output.
#[derive(Debug, Clone, Serialize)]
pub struct Rotated {
    /// The token file rewritten (0600).
    pub token_path: String,
    /// Whether there was a token before.
    pub replaced: bool,
    /// `credentials.json` exists and was left as it was.
    pub credentials_kept: bool,
    /// `HARBARD_HUB_TOKEN` is set in this shell, and wins over the file for
    /// whatever reads it.
    pub env_override: bool,
}

/// `harbard hub rotate`: a new shared hub token, written 0600 by rename.
/// Only the token file changes: per-device credentials, the device
/// registry and any satellite pairing stay as they are. The token itself
/// is never printed (`harbard hub invite` prints the pairing line).
pub fn rotate_hub_token(store: &Store) -> io::Result<Rotated> {
    let replaced = read_hub_token(store).is_some();
    write_hub_token(store, &generate_hub_token())?;
    Ok(Rotated {
        token_path: hub_token_path(store).display().to_string(),
        replaced,
        credentials_kept: credentials_path(store).is_file(),
        env_override: std::env::var("HARBARD_HUB_TOKEN").is_ok_and(|t| !t.trim().is_empty()),
    })
}

/// What to restart after a rotation, and who must pair again.
pub fn rotated_text(r: &Rotated) -> String {
    let mut out = format!(
        "harbard hub rotate: a new hub token is in {} (0600){}.\n",
        r.token_path,
        if r.replaced {
            "; the old one stops working once serve restarts"
        } else {
            ""
        }
    );
    out.push_str(
        "Restart serve now, which reads the token when it starts: launchctl kickstart -k gui/$(id -u)/com.harbard.serve\n",
    );
    out.push_str(
        "The console and the desktop app need no restart: they read the token file on each request, so until serve restarts they are refused.\n",
    );
    if r.credentials_kept {
        out.push_str(
            "credentials.json left untouched: devices paired with their own credential keep working.\n",
        );
    } else {
        out.push_str(
            "No credentials.json: every laptop paired with the old shared token must pair again (harbard hub invite prints the new line; run harbard pair '…' there).\n",
        );
    }
    if r.env_override {
        out.push_str(
            "HARBARD_HUB_TOKEN is set in this shell: whatever reads it ignores the file until it is unset or updated.\n",
        );
    }
    out.trim_end().to_string()
}

/// Two ULIDs — enough entropy, no extra deps, copy-paste friendly.
pub fn generate_hub_token() -> String {
    format!("{}{}", Ulid::generate(), Ulid::generate())
}

/// Best-effort primary LAN IPv4 (not loopback). Used for invite URLs without
/// probing an external route.
pub fn lan_ipv4() -> Option<String> {
    use nix::net::if_::InterfaceFlags;

    let interfaces = nix::ifaddrs::getifaddrs().ok()?;
    for interface in interfaces {
        if !interface.flags.contains(InterfaceFlags::IFF_UP)
            || interface.flags.contains(InterfaceFlags::IFF_LOOPBACK)
        {
            continue;
        }
        let Some(address) = interface.address else {
            continue;
        };
        if let Some(v4) = address.as_sockaddr_in() {
            let ip = v4.ip();
            if !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified() {
                return Some(ip.to_string());
            }
        }
    }
    None
}

/// Parse `http://host:7419#token` from `harbard hub invite`.
pub fn parse_pair_invite(raw: &str) -> Result<(String, String), String> {
    let raw = raw.trim().trim_matches(|c| c == '\'' || c == '"');
    if raw.is_empty() {
        return Err("empty invite".into());
    }
    let (url, token) = match raw.split_once('#') {
        Some((u, t)) => (u.trim(), t.trim()),
        None => {
            return Err(
                "invite must look like http://<hub-ip>:7419#<token> (copy from `harbard hub invite`)"
                    .into(),
            );
        }
    };
    if token.is_empty() {
        return Err("invite is missing the #token part".into());
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!("hub URL must start with http(s):// — got {url}"));
    }
    Ok((url.trim_end_matches('/').to_string(), token.to_string()))
}

pub fn launchd_serve_plist() -> PathBuf {
    home_dir().join("Library/LaunchAgents/com.harbard.serve.plist")
}

pub fn launchd_satellite_plist() -> PathBuf {
    home_dir().join("Library/LaunchAgents/com.harbard.satellite.plist")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn which_harbard() -> String {
    let cargo = home_dir().join(".cargo/bin/harbard");
    if cargo.is_file() {
        return cargo.display().to_string();
    }
    "harbard".into()
}

fn users_uid() -> u32 {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(501)
}

fn path_env(home: &str) -> String {
    format!("{home}/.cargo/bin:{home}/.bun/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin")
}

/// Rewrite `com.harbard.serve` for hub mode (or restore loopback).
///
/// Refuses when `HARBARD_HOME` points away from the default store — a custom
/// home is for tests/isolation and must not clobber the operator's LaunchAgent.
pub fn write_serve_launchd(port: u16, lan: bool, _token: Option<&str>) -> io::Result<PathBuf> {
    guard_default_home_for_launchd()?;
    let path = launchd_serve_plist();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let home = home_dir().display().to_string();
    let harbard = xml_escape(&which_harbard());
    let mut args = format!(
        "\t\t<string>{harbard}</string>\n\t\t<string>serve</string>\n\t\t<string>--port</string>\n\t\t<string>{port}</string>\n"
    );
    if lan {
        // Loopback plus the tailnet address, when configured. Never 0.0.0.0.
        args.push_str("\t\t<string>--bind</string>\n\t\t<string>tailnet</string>\n");
    }
    let path_val = xml_escape(&path_env(&home));
    // `serve` resolves the token from ~/.harbard/hub.token. Do not duplicate a
    // bearer credential into a LaunchAgent plist where process tooling and
    // backups may expose it.
    let env = format!(
        "\t<key>EnvironmentVariables</key>\n\t<dict>\n\t\t<key>PATH</key>\n\t\t<string>{path_val}</string>\n\t</dict>\n"
    );
    let home_xml = xml_escape(&home);
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>com.harbard.serve</string>
	<key>ProgramArguments</key>
	<array>
{args}	</array>
{env}	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<true/>
	<key>ThrottleInterval</key>
	<integer>10</integer>
	<key>ProcessType</key>
	<string>Background</string>
	<key>WorkingDirectory</key>
	<string>{home_xml}</string>
	<key>StandardOutPath</key>
	<string>{home_xml}/Library/Logs/harbard-serve.log</string>
	<key>StandardErrorPath</key>
	<string>{home_xml}/Library/Logs/harbard-serve.log</string>
</dict>
</plist>
"#
    );
    write_private(&path, plist)?;
    Ok(path)
}

pub fn reload_launchd_label(label: &str, plist: &Path) -> io::Result<()> {
    let uid = users_uid();
    let domain = format!("gui/{uid}");
    let full = format!("{domain}/{label}");
    let plist_s = plist
        .to_str()
        .ok_or_else(|| io::Error::other("plist path not utf-8"))?;

    // Prefer kickstart-in-place when already loaded; only bootout+bootstrap
    // when the service is missing. Blind bootout races KeepAlive and can leave
    // the operator without serve until the next manual bootstrap.
    let printed = Command::new("launchctl")
        .args(["print", &full])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if printed {
        let kick = Command::new("launchctl")
            .args(["kickstart", "-k", &full])
            .status()?;
        if kick.success() {
            return Ok(());
        }
    }

    let _ = Command::new("launchctl").args(["bootout", &full]).status();
    let boot = Command::new("launchctl")
        .args(["bootstrap", &domain, plist_s])
        .status()?;
    if !boot.success() {
        return Err(io::Error::other(format!(
            "launchctl bootstrap failed for {label}"
        )));
    }
    let _ = Command::new("launchctl").args(["enable", &full]).status();
    let kick = Command::new("launchctl")
        .args(["kickstart", "-k", &full])
        .status()?;
    if !kick.success() {
        return Err(io::Error::other(format!(
            "launchctl kickstart failed for {label}"
        )));
    }
    Ok(())
}

pub fn kickstart_serve() -> io::Result<()> {
    reload_launchd_label("com.harbard.serve", &launchd_serve_plist())
}

/// Write a LaunchAgent that keeps `harbard satellite` alive on a laptop.
pub fn write_satellite_launchd(_store: &Store) -> io::Result<PathBuf> {
    guard_default_home_for_launchd()?;
    let path = launchd_satellite_plist();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let home = home_dir().display().to_string();
    let harbard = xml_escape(&which_harbard());
    // The satellite process loads hub, token, and device identity from the
    // mode-0600 satellite.json. Keep secrets out of LaunchAgent metadata.
    let path_val = xml_escape(&path_env(&home));
    let home_xml = xml_escape(&home);
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>com.harbard.satellite</string>
	<key>ProgramArguments</key>
	<array>
		<string>{harbard}</string>
		<string>satellite</string>
	</array>
	<key>EnvironmentVariables</key>
	<dict>
		<key>PATH</key>
		<string>{path_val}</string>
	</dict>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<true/>
	<key>ThrottleInterval</key>
	<integer>15</integer>
	<key>WorkingDirectory</key>
	<string>{home_xml}</string>
	<key>StandardOutPath</key>
	<string>{home_xml}/Library/Logs/harbard-satellite.log</string>
	<key>StandardErrorPath</key>
	<string>{home_xml}/Library/Logs/harbard-satellite.log</string>
</dict>
</plist>
"#
    );
    write_private(&path, plist)?;
    Ok(path)
}

pub fn kickstart_satellite() -> io::Result<()> {
    reload_launchd_label("com.harbard.satellite", &launchd_satellite_plist())
}

pub fn stop_satellite_launchd() -> io::Result<()> {
    let uid = users_uid();
    let full = format!("gui/{uid}/com.harbard.satellite");
    let _ = Command::new("launchctl").args(["bootout", &full]).status();
    let path = launchd_satellite_plist();
    if path.is_file() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

/// The pair line. The host is the tailnet address when one is configured;
/// without it only this Mac reaches serve, so the line names loopback.
pub fn invite_line(tailnet: Option<&str>, port: u16, token: &str) -> String {
    let host = match tailnet {
        Some(h) if h.contains(':') => format!("[{h}]"),
        Some(h) => h.to_string(),
        None => "127.0.0.1".to_string(),
    };
    format!("http://{host}:{port}#{token}")
}

pub fn print_invite_block(tailnet: Option<&str>, port: u16, token: &str, name: &str, role: &str) {
    let invite = invite_line(tailnet, port, token);
    println!("# {name} · {role} · this invite pairs one device, once, within a day.");
    println!("# On that device:");
    println!("harbard pair '{invite}'");
    println!();
    if tailnet.is_none() {
        println!("# No tailnet address is configured, so serve listens on this Mac only.");
        println!("# The line works for a satellite simulated here; another machine needs");
        println!("# the tailnet bind first.");
        println!();
    }
    println!("# The token is shown once. The hub keeps only its hash; the device keeps it");
    println!("# in ~/.harbard/satellite.json (0600). Revoke: harbard hub revoke {name}");
}

pub fn file_exists(path: &Path) -> bool {
    path.is_file()
}

fn guard_default_home_for_launchd() -> io::Result<()> {
    let Ok(custom) = std::env::var("HARBARD_HOME") else {
        return Ok(());
    };
    let custom = custom.trim();
    if custom.is_empty() {
        return Ok(());
    }
    let default = home_dir().join(".harbard");
    let custom_path = PathBuf::from(custom);
    let same = custom_path
        .canonicalize()
        .ok()
        .zip(default.canonicalize().ok())
        .map(|(a, b)| a == b)
        .unwrap_or(false);
    if same {
        return Ok(());
    }
    Err(io::Error::other(
        "refusing to rewrite LaunchAgents while HARBARD_HOME is set to a non-default path (avoids clobbering the operator agent from tests)",
    ))
}

/// Fleet lines for `harbard doctor` (human).
pub fn doctor_fleet_text(store: &Store) -> String {
    let mut lines = Vec::new();
    let token = read_hub_token(store);
    let sat = SatelliteConfig::load(store);
    let reg = harbard_core::DeviceRegistry::load(store);

    if let Some(cfg) = &sat {
        lines.push(format!(
            "fleet        satellite → {} (device {})",
            cfg.hub,
            if cfg.device_id.is_empty() {
                "unset"
            } else {
                &cfg.device_id
            }
        ));
    } else if token.is_some() {
        let host = lan_ipv4().unwrap_or_else(|| "lan?".into());
        lines.push(format!(
            "fleet        hub token present · invite host {host}:7419 · {} device(s)",
            reg.devices.len()
        ));
    } else {
        lines.push(
            "fleet        local-only (loopback). `harbard hub enable` to reach other machines."
                .into(),
        );
    }

    if let Some(dev) = read_local_device_id(store) {
        lines.push(format!("device       {dev}"));
    }
    lines.join("\n")
}

pub fn doctor_fleet_json(store: &Store) -> serde_json::Value {
    let sat = SatelliteConfig::load(store);
    let token = read_hub_token(store);
    let reg = harbard_core::DeviceRegistry::load(store);
    serde_json::json!({
        "hub_token": token.is_some(),
        "satellite": sat.as_ref().map(|c| serde_json::json!({
            "hub": c.hub,
            "device_id": c.device_id,
            "name": c.name,
        })),
        "lan_ip": lan_ipv4(),
        "devices": reg.devices.len(),
        "local_device_id": read_local_device_id(store),
        "serve_plist": file_exists(&launchd_serve_plist()),
        "satellite_plist": file_exists(&launchd_satellite_plist()),
    })
}

fn read_local_device_id(store: &Store) -> Option<String> {
    let raw = fs::read_to_string(store.root().join("device.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("id")
        .and_then(|id| id.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_invite_happy() {
        let (u, t) = parse_pair_invite("http://192.168.1.10:7419#AbCdEf").unwrap();
        assert_eq!(u, "http://192.168.1.10:7419");
        assert_eq!(t, "AbCdEf");
    }

    #[test]
    fn parse_invite_strips_quotes() {
        let (u, t) = parse_pair_invite("'http://h:7419#tok'").unwrap();
        assert_eq!(u, "http://h:7419");
        assert_eq!(t, "tok");
    }

    #[test]
    fn parse_invite_rejects_bare_url() {
        assert!(parse_pair_invite("http://192.168.1.10:7419").is_err());
    }

    #[test]
    fn generate_token_is_long() {
        assert!(generate_hub_token().len() >= 40);
    }
}
