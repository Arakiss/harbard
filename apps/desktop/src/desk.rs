//! The live state every screen reads: the workspace view, the faces slice,
//! the connection, and the one door for writes.
//!
//! `Desk` is a GPUI entity installed as a global handle ([`DeskHandle`]).
//! Screens read it in `render` (`Desk::get(cx).read(cx)`) and re-render when
//! it notifies, which happens once per hub read that changed something.

use crate::api;
use crate::hub::{Hub, HubMsg};
use crate::model::{ActivityView, WorkspaceView};
use gpui::{App, AppContext, Context, Entity, Global, Task};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How the app last heard from the bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    Connecting,
    Connected,
    Disconnected,
}

pub struct Desk {
    pub hub: Hub,
    pub view: Option<Arc<WorkspaceView>>,
    pub activity: Option<Arc<ActivityView>>,
    pub error: Option<String>,
    /// Wall clock when `view` arrived; `now_ms()` carries serve's clock forward.
    pub at: Instant,
    /// How long the last full read took to compute.
    pub took: Duration,
    /// The workspace the reads are for (`None` = the hub's default).
    pub workspace: Option<String>,
}

#[derive(Clone)]
pub struct DeskHandle(pub Entity<Desk>);

impl Global for DeskHandle {}

impl Desk {
    pub fn get(cx: &App) -> Entity<Desk> {
        cx.global::<DeskHandle>().0.clone()
    }

    /// Start the hub and the task that applies its reads.
    pub fn init(cx: &mut App) {
        let (tx, rx) = async_channel::unbounded::<HubMsg>();
        let hub = Hub::start(tx);
        let workspace = saved_workspace();
        let desk = cx.new(|cx: &mut Context<Desk>| {
            cx.spawn(async move |this, cx| {
                while let Ok(msg) = rx.recv().await {
                    let alive = this.update(cx, |desk, cx| desk.apply(msg, cx));
                    if alive.is_err() {
                        break;
                    }
                }
            })
            .detach();
            Desk {
                hub,
                view: None,
                activity: None,
                error: None,
                at: Instant::now(),
                took: Duration::ZERO,
                workspace,
            }
        });
        cx.set_global(DeskHandle(desk));
    }

    fn apply(&mut self, msg: HubMsg, cx: &mut Context<Self>) {
        if std::env::var_os("HARBARD_APP_TRACE").is_some() {
            eprintln!(
                "desk {:?}: {}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis(),
                match &msg {
                    HubMsg::Full(_, t) => format!("full {t:?}"),
                    HubMsg::Activity(_) => "activity".into(),
                    HubMsg::Failed(e) => format!("failed {e}"),
                }
            );
        }
        match msg {
            HubMsg::Full(view, took) => {
                if self.view.as_deref() != Some(&*view) {
                    self.view = Some(view);
                    self.at = Instant::now();
                    self.took = took;
                    self.error = None;
                    crate::perf::count(crate::perf::Source::Bus, 1);
                    cx.notify();
                } else if self.error.take().is_some() {
                    cx.notify();
                }
            }
            HubMsg::Activity(a) => {
                if self.activity.as_deref() != Some(&*a) {
                    self.activity = Some(a);
                    crate::perf::count(crate::perf::Source::Bus, 1);
                    cx.notify();
                }
            }
            // The workspace on screen is gone (removed here, from the CLI or
            // from another desk): read the default instead of failing on.
            HubMsg::Failed(e)
                if self.workspace.is_some()
                    && crate::derive::workspaces::read_lost_workspace(&e) =>
            {
                self.switch_workspace(None, cx);
            }
            HubMsg::Failed(e) => {
                self.error = Some(e);
                cx.notify();
            }
        }
    }

    pub fn connection(&self) -> Connection {
        match (&self.view, &self.error) {
            (None, None) => Connection::Connecting,
            (_, Some(_)) => Connection::Disconnected,
            _ => Connection::Connected,
        }
    }

    /// serve's clock carried forward from the last read.
    pub fn now_ms(&self) -> u64 {
        let base = self.view.as_ref().map_or(0, |v| v.now_ms);
        if base == 0 {
            return harbard_core::clock::now_ms();
        }
        base + self.at.elapsed().as_millis() as u64
    }

    /// Switch workspace: remembered in `desk.json`, re-read at once.
    pub fn switch_workspace(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.workspace = id.clone();
        save_desk_prefs(|p| p.workspace = id.clone());
        self.hub.set_workspace(id);
        cx.notify();
    }

    /// `POST /api/v1/workspace/<action>` with the current workspace stamped
    /// in, then a fresh read. Resolves to serve's answer.
    pub fn post(
        &self,
        action: &str,
        mut body: Value,
        cx: &mut App,
    ) -> Task<Result<Value, api::ApiError>> {
        if let (Some(ws), Some(obj)) = (&self.workspace, body.as_object_mut())
            && !obj.contains_key("workspace")
            && ws != "all"
        {
            obj.insert("workspace".into(), json!(ws));
        }
        let path = format!("/workspace/{action}");
        let hub = self.hub.clone();
        let refresh = action != "open";
        cx.background_spawn(async move {
            let out = api::post(&path, &body);
            if refresh {
                hub.refresh();
            }
            out
        })
    }

    /// Any other write to serve, followed by a fresh read.
    pub fn post_to(
        &self,
        path: &str,
        body: Value,
        cx: &mut App,
    ) -> Task<Result<Value, api::ApiError>> {
        let path = path.to_string();
        let hub = self.hub.clone();
        cx.background_spawn(async move {
            let out = api::post(&path, &body);
            hub.refresh();
            out
        })
    }
}

/// The app's own preferences, `~/.harbard/desk.json` (shared with the old
/// desktop shell, which kept `mute` and `locale` there).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeskPrefs {
    #[serde(default)]
    pub mute: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(flatten)]
    pub rest: serde_json::Map<String, Value>,
}

fn desk_path() -> std::path::PathBuf {
    harbard_core::store::Store::resolve()
        .root()
        .join("desk.json")
}

pub fn desk_prefs() -> DeskPrefs {
    std::fs::read(desk_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Read, change and write back `desk.json` (write to a temp file, then rename).
pub fn save_desk_prefs(change: impl FnOnce(&mut DeskPrefs)) {
    let mut prefs = desk_prefs();
    change(&mut prefs);
    let path = desk_path();
    let tmp = path.with_extension("json.tmp");
    if let Ok(bytes) = serde_json::to_vec_pretty(&prefs)
        && std::fs::write(&tmp, bytes).is_ok()
    {
        let _ = std::fs::rename(&tmp, &path);
    }
}

pub fn saved_workspace() -> Option<String> {
    desk_prefs().workspace
}
