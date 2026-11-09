//! A fixed sentence serve sends with enough to word it again elsewhere.
//!
//! The sentence itself is English and is what the CLI and any client that
//! knows no better show. Beside it travels a stable `code` and its `params`,
//! so the console can put the same sentence in the operator's language from
//! its dictionaries, and fall back on the English for a code it does not know.

use serde::Serialize;
use std::borrow::Cow;
use std::collections::BTreeMap;

/// A value slotted into a coded sentence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Param {
    Text(String),
    Number(u64),
}

impl From<&str> for Param {
    fn from(value: &str) -> Self {
        Param::Text(value.to_string())
    }
}

impl From<String> for Param {
    fn from(value: String) -> Self {
        Param::Text(value)
    }
}

impl From<u64> for Param {
    fn from(value: u64) -> Self {
        Param::Number(value)
    }
}

impl From<usize> for Param {
    fn from(value: usize) -> Self {
        Param::Number(value as u64)
    }
}

/// The values of one coded sentence, by name.
pub type Params = BTreeMap<Cow<'static, str>, Param>;

/// The params of a coded sentence, written at the call site.
pub fn params<const N: usize>(entries: [(&'static str, Param); N]) -> Params {
    entries
        .into_iter()
        .map(|(name, value)| (Cow::Borrowed(name), value))
        .collect()
}
