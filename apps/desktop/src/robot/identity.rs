//! Which robot an agent is: the console's `identity.ts`, `robots.ts` and
//! `season-robots.tsx`. The same name picks the same model, light and tile
//! here as in the console, on every machine.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Pixel sizes rendered for every body and mask, smallest first.
pub const ROBOT_SIZES: [u32; 5] = [36, 72, 144, 216, 432];

/// A screen's bounding box, as fractions of the tile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RobotModel {
    pub id: &'static str,
    pub screen: ScreenBox,
}

const fn model(id: &'static str, x: f32, y: f32, w: f32, h: f32) -> RobotModel {
    RobotModel {
        id,
        screen: ScreenBox { x, y, w, h },
    }
}

/// In order: the name hash indexes into this list (generated with the art).
pub const ROBOT_MODELS: [RobotModel; 12] = [
    model("e-off", 0.10367, 0.18581, 0.52153, 0.41866),
    model("antenna", 0.12679, 0.22488, 0.49841, 0.38836),
    model("dome", 0.10686, 0.18979, 0.51834, 0.41308),
    model("dials", 0.10447, 0.18581, 0.52073, 0.39553),
    model("radio", 0.10367, 0.1866, 0.52153, 0.41707),
    model("teapot", 0.13078, 0.20893, 0.49442, 0.38995),
    model("headset", 0.10845, 0.1866, 0.51675, 0.41707),
    model("propeller", 0.10526, 0.19617, 0.51754, 0.40829),
    model("lantern", 0.10686, 0.24083, 0.50877, 0.37719),
    model("bowler", 0.10447, 0.18501, 0.52073, 0.41946),
    model("flower", 0.10367, 0.18581, 0.52153, 0.41866),
    model("stripes", 0.10447, 0.18581, 0.52073, 0.41866),
];

/// The screen light of a robot; blocked and waiting faces turn amber.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Glow {
    Warm,
    Ice,
    Mint,
    Lilac,
    Amber,
}

impl Glow {
    pub const NAMED: [Glow; 4] = [Glow::Warm, Glow::Ice, Glow::Mint, Glow::Lilac];

    /// The theme token (`--hb-robot-glow-<id>`).
    pub fn token(self) -> &'static str {
        match self {
            Glow::Warm => "robot-glow-warm",
            Glow::Ice => "robot-glow-ice",
            Glow::Mint => "robot-glow-mint",
            Glow::Lilac => "robot-glow-lilac",
            Glow::Amber => "robot-glow-amber",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Glow::Warm => "warm",
            Glow::Ice => "ice",
            Glow::Mint => "mint",
            Glow::Lilac => "lilac",
            Glow::Amber => "amber",
        }
    }
}

/// One agent's robot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RobotIdentity {
    pub model: RobotModel,
    pub glow: Glow,
    /// 1..=4: `--hb-robot-tile-<n>`.
    pub tile: u32,
    /// Per-agent timing, so idle breathing and blinking never march in step.
    pub blink_s: f32,
    pub blink_delay_s: f32,
    pub breathe_delay_s: f32,
}

/// FNV-1a over code points, as the console hashes a name.
pub fn name_hash(name: &str) -> u32 {
    let mut x: u32 = 2_166_136_261;
    for c in name.chars() {
        x ^= c as u32;
        x = x.wrapping_mul(16_777_619);
    }
    x
}

pub fn robot_for(name: &str) -> RobotIdentity {
    let h = name_hash(name);
    RobotIdentity {
        model: ROBOT_MODELS[(h % ROBOT_MODELS.len() as u32) as usize],
        glow: Glow::NAMED[((h >> 5) % 4) as usize],
        tile: ((h >> 9) % 4) + 1,
        blink_s: 4.4 + ((h >> 13) % 21) as f32 / 10.0,
        blink_delay_s: -(((h >> 18) % 50) as f32 / 10.0),
        breathe_delay_s: -(((h >> 23) % 46) as f32 / 10.0),
    }
}

/// The smallest rendered size that covers `css` points at `scale`.
pub fn asset_size(css: f32, scale: f32) -> u32 {
    let need = css * scale;
    ROBOT_SIZES
        .iter()
        .copied()
        .find(|&s| s as f32 >= need)
        .unwrap_or(ROBOT_SIZES[ROBOT_SIZES.len() - 1])
}

/// The art folder under the bundled assets: `robots` or `robots/<season>`.
pub fn art_dir(season: Option<&str>) -> String {
    match season {
        Some(s) => format!("robots/{s}"),
        None => "robots".into(),
    }
}

pub fn body_path(dir: &str, model: &str, size: u32) -> String {
    format!("{dir}/{model}-{size}.webp")
}

pub fn mask_path(dir: &str, model: &str, size: u32) -> String {
    format!("{dir}/{model}-mask-{size}.png")
}

fn fraction(v: &serde_json::Value) -> Option<f32> {
    let f = v.as_f64()?;
    (f.is_finite() && (0.0..=1.0).contains(&f)).then_some(f as f32)
}

/// The variants in a season's `models.json` that name a known model and a
/// screen inside the tile; the rest are dropped.
pub fn parse_season_models(raw: &serde_json::Value) -> HashMap<&'static str, RobotModel> {
    let mut out = HashMap::new();
    let Some(items) = raw.as_array() else {
        return out;
    };
    for item in items {
        let Some(id) = item.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(known) = ROBOT_MODELS.iter().find(|m| m.id == id) else {
            continue;
        };
        let Some(screen) = item.get("screen") else {
            continue;
        };
        let (Some(x), Some(y), Some(w), Some(h)) = (
            screen.get("x").and_then(fraction),
            screen.get("y").and_then(fraction),
            screen.get("w").and_then(fraction),
            screen.get("h").and_then(fraction),
        ) else {
            continue;
        };
        out.insert(
            known.id,
            RobotModel {
                id: known.id,
                screen: ScreenBox { x, y, w, h },
            },
        );
    }
    out
}

/// A season id is a folder name: lowercase letters, digits and dashes.
pub fn valid_season(season: &str) -> bool {
    !season.is_empty()
        && season
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

type SeasonMap = HashMap<String, Arc<HashMap<&'static str, RobotModel>>>;

/// A season's variants, read once from the bundled `models.json`; a season
/// without art has none.
pub fn season_models(season: &str) -> Arc<HashMap<&'static str, RobotModel>> {
    static LOADS: OnceLock<Mutex<SeasonMap>> = OnceLock::new();
    let loads = LOADS.get_or_init(Default::default);
    let mut loads = loads.lock().unwrap_or_else(|e| e.into_inner());
    loads
        .entry(season.to_string())
        .or_insert_with(|| {
            if !valid_season(season) {
                return Arc::new(HashMap::new());
            }
            let raw = crate::assets::Assets::local(&format!("robots/{season}/models.json"))
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .unwrap_or(serde_json::Value::Null);
            Arc::new(parse_season_models(&raw))
        })
        .clone()
}

/// The model to draw and the folder its art is in: the season's variant
/// when there is one, else the base model.
pub fn dressed(model: RobotModel, season: Option<&str>) -> (RobotModel, String) {
    if let Some(season) = season {
        let variants = season_models(season);
        if let Some(variant) = variants.get(model.id) {
            return (*variant, art_dir(Some(season)));
        }
    }
    (model, art_dir(None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tile_hex(n: u32) -> &'static str {
        ["#dfe8d8", "#e6eef2", "#f2e6dc", "#e8e3f2"][(n - 1) as usize]
    }

    const EXPECTED: [(&str, &str, &str, &str); 11] = [
        ("keychain", "antenna", "mint", "#e8e3f2"),
        ("harbard-themes", "e-off", "mint", "#f2e6dc"),
        ("identity", "propeller", "warm", "#f2e6dc"),
        ("parser", "flower", "ice", "#e8e3f2"),
        ("kc-review-2", "bowler", "ice", "#f2e6dc"),
        ("mind-backup", "flower", "lilac", "#e6eef2"),
        ("harbard-chat", "lantern", "ice", "#f2e6dc"),
        ("keychain-review", "e-off", "lilac", "#f2e6dc"),
        ("harbard-view", "dials", "ice", "#dfe8d8"),
        ("recall-perf", "lantern", "warm", "#e6eef2"),
        ("ñandú-ü", "lantern", "mint", "#f2e6dc"),
    ];

    #[test]
    fn the_same_name_always_picks_the_same_robot() {
        for (name, model, glow, tile) in EXPECTED {
            let r = robot_for(name);
            assert_eq!(
                (name, r.model.id, r.glow.id(), tile_hex(r.tile)),
                (name, model, glow, tile)
            );
            assert_eq!(robot_for(name), r);
        }
    }

    #[test]
    fn models_are_twelve_in_order_and_every_asset_exists() {
        let ids: Vec<&str> = ROBOT_MODELS.iter().map(|m| m.id).collect();
        assert_eq!(
            ids,
            [
                "e-off",
                "antenna",
                "dome",
                "dials",
                "radio",
                "teapot",
                "headset",
                "propeller",
                "lantern",
                "bowler",
                "flower",
                "stripes"
            ]
        );
        for m in ROBOT_MODELS {
            for s in ROBOT_SIZES {
                assert!(
                    crate::assets::Assets::local(&body_path("robots", m.id, s)).is_some(),
                    "{} {s}",
                    m.id
                );
                assert!(
                    crate::assets::Assets::local(&mask_path("robots", m.id, s)).is_some(),
                    "{} mask {s}",
                    m.id
                );
            }
        }
    }

    #[test]
    fn idle_breathing_and_blinking_are_desynchronised_per_agent() {
        let timings: std::collections::HashSet<String> = EXPECTED
            .iter()
            .map(|(n, ..)| {
                let r = robot_for(n);
                format!("{}|{}|{}", r.blink_s, r.blink_delay_s, r.breathe_delay_s)
            })
            .collect();
        assert!(timings.len() > EXPECTED.len() - 2);
    }

    #[test]
    fn a_36_point_robot_uses_the_72_art_at_2x_a_150_one_the_432() {
        let sizes: Vec<u32> = [24.0, 36.0, 48.0, 64.0, 108.0, 150.0]
            .iter()
            .map(|&c| asset_size(c, 2.0))
            .collect();
        assert_eq!(sizes, [72, 72, 144, 144, 216, 432]);
        assert_eq!(asset_size(36.0, 1.0), 36);
        assert_eq!(
            mask_path(
                &art_dir(Some("christmas")),
                "antenna",
                asset_size(36.0, 2.0)
            ),
            "robots/christmas/antenna-mask-72.png"
        );
        assert_eq!(
            mask_path(&art_dir(None), "antenna", 72),
            "robots/antenna-mask-72.png"
        );
    }

    #[test]
    fn a_season_list_keeps_known_models_with_a_screen_inside_the_tile() {
        let models = parse_season_models(&json!([
            { "id": "antenna", "screen": { "x": 0.2, "y": 0.38, "w": 0.4, "h": 0.31 } },
            { "id": "unknown-model", "screen": { "x": 0.1, "y": 0.1, "w": 0.5, "h": 0.5 } },
            { "id": "dome", "screen": { "x": 0.1, "y": 0.1, "w": 1.5, "h": 0.5 } },
            { "id": "../dials", "screen": { "x": 0.1, "y": 0.1, "w": 0.5, "h": 0.5 } },
            { "id": "radio" },
            "teapot",
        ]));
        assert_eq!(models.keys().copied().collect::<Vec<_>>(), ["antenna"]);
        assert_eq!(
            models["antenna"].screen,
            ScreenBox {
                x: 0.2,
                y: 0.38,
                w: 0.4,
                h: 0.31
            }
        );
        assert!(parse_season_models(&json!({ "antenna": {} })).is_empty());
        assert!(parse_season_models(&serde_json::Value::Null).is_empty());
    }

    #[test]
    fn a_season_without_art_has_no_variants_and_a_bad_id_reads_nothing() {
        assert!(season_models("no-art-season").is_empty());
        assert!(season_models("../etc").is_empty());
        assert!(!valid_season("../etc"));
    }

    #[test]
    fn every_bundled_season_is_whole() {
        let seasons: std::collections::BTreeSet<String> =
            crate::assets::Assets::local_paths("robots/")
                .into_iter()
                .filter_map(|p| {
                    p.strip_suffix("/models.json")
                        .and_then(|d| d.strip_prefix("robots/"))
                        .map(str::to_string)
                })
                .collect();
        assert!(seasons.contains("christmas"));
        for season in seasons {
            let raw: serde_json::Value = serde_json::from_slice(
                &crate::assets::Assets::local(&format!("robots/{season}/models.json")).unwrap(),
            )
            .unwrap();
            let models = parse_season_models(&raw);
            assert_eq!(
                models.len(),
                raw.as_array().map_or(usize::MAX, Vec::len),
                "{season}"
            );
            let dir = art_dir(Some(&season));
            for id in models.keys() {
                for s in ROBOT_SIZES {
                    assert!(
                        crate::assets::Assets::local(&body_path(&dir, id, s)).is_some(),
                        "{season} {id} {s}"
                    );
                    assert!(
                        crate::assets::Assets::local(&mask_path(&dir, id, s)).is_some(),
                        "{season} {id} mask {s}"
                    );
                }
            }
        }
    }
}
