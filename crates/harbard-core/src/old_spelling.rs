//! Words older builds stored in Spanish, read back in today's spelling.
//!
//! Real installs keep `workspace.json`, `profile.json`, `digests.json`,
//! attachment manifests and feedback events under the harbard home, and the
//! event log is append-only, so what it holds can never be rewritten. Every
//! function in this module exists only to read those old files: nothing
//! writes these spellings any more, and a file read through them is saved
//! back in the new spelling the next time it is written.
//!
//! Each `read_*` is the serde adapter of the function above it, so the old
//! word cannot reach the rest of the program.

use serde::{Deserialize, Deserializer};

/// A coordination state (`profile.json`, `digests.json`).
pub fn coordination_state(stored: &str) -> &str {
    match stored {
        "disponible" => "available",
        "concentrado" => "focused",
        "saturado" => "swamped",
        "fuera" => "away",
        "revisando" => "reviewing",
        "fin" => "done",
        other => other,
    }
}

/// What ended a held-notes digest (`digests.json`).
pub fn digest_reason(stored: &str) -> &str {
    match stored {
        "revisando" => "reviewing",
        other => other,
    }
}

/// An activity state, as a feedback sample recorded it.
pub fn activity_state(stored: &str) -> &str {
    match stored {
        "pensando" => "thinking",
        "ejecutando" => "running",
        "pruebas" => "testing",
        "editando" => "editing",
        "leyendo" => "reading",
        "delegando" => "delegating",
        "esperando" => "waiting",
        "dormido" => "asleep",
        "bloqueado" => "blocked",
        "terminado" => "finished",
        "verificado" => "verified",
        "desconectado" => "disconnected",
        other => other,
    }
}

/// The theme mode in `workspace.json`.
pub fn theme_mode(stored: &str) -> &str {
    match stored {
        "claro" => "light",
        "oscuro" => "dark",
        "sistema" => "system",
        other => other,
    }
}

/// A built-in theme id in `workspace.json`. A custom theme is the name of
/// its file, which stays as the operator wrote it.
pub fn theme_id(stored: &str) -> &str {
    match stored {
        "berenjena" => "aubergine",
        "clementina" => "clementine",
        "laguna" => "lagoon",
        "grafito" => "graphite",
        "fiordo" => "fjord",
        other => other,
    }
}

/// The motion preference in `workspace.json`.
pub fn motion(stored: &str) -> &str {
    match stored {
        "reducir" => "reduce",
        other => other,
    }
}

/// The duration "Do not disturb" offers first, in `profile.json`.
pub fn dnd_default(stored: &str) -> &str {
    match stored {
        "manana" => "tomorrow",
        other => other,
    }
}

/// An attachment label in a manifest, accents optional, in today's spelling.
pub fn attachment_label(stored: &str) -> &str {
    match stored {
        "antes" => "before",
        "después" | "despues" => "after",
        "informe" => "report",
        "código" | "codigo" => "code",
        other => other,
    }
}

/// Serde adapter of [`coordination_state`], only for reading old files.
pub fn read_coordination_state<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(coordination_state(&String::deserialize(d)?).to_string())
}

/// Serde adapter of [`digest_reason`], only for reading old files.
pub fn read_digest_reason<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(digest_reason(&String::deserialize(d)?).to_string())
}

/// Serde adapter of [`activity_state`], only for reading old files.
pub fn read_activity_state<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(activity_state(&String::deserialize(d)?).to_string())
}

/// Serde adapter of [`theme_mode`], only for reading old files.
pub fn read_theme_mode<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(d)?.map(|m| theme_mode(&m).to_string()))
}

/// Serde adapter of [`theme_id`], only for reading old files.
pub fn read_theme_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(d)?.map(|t| theme_id(&t).to_string()))
}

/// Serde adapter of [`motion`], only for reading old files.
pub fn read_motion<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(d)?.map(|m| motion(&m).to_string()))
}

/// Serde adapter of [`dnd_default`], only for reading old files.
pub fn read_dnd_default<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(d)?.map(|m| dnd_default(&m).to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_old_word_has_a_new_one_and_today_spelling_passes_through() {
        assert_eq!(coordination_state("concentrado"), "focused");
        assert_eq!(coordination_state("focused"), "focused");
        assert_eq!(activity_state("pensando"), "thinking");
        assert_eq!(activity_state("thinking"), "thinking");
        assert_eq!(theme_mode("sistema"), "system");
        assert_eq!(theme_id("berenjena"), "aubergine");
        assert_eq!(theme_id("mi-tema"), "mi-tema");
        assert_eq!(motion("reducir"), "reduce");
        assert_eq!(dnd_default("manana"), "tomorrow");
        assert_eq!(attachment_label("después"), "after");
        assert_eq!(attachment_label("despues"), "after");
        assert_eq!(digest_reason("revisando"), "reviewing");
        assert_eq!(digest_reason("left"), "left");
    }
}
