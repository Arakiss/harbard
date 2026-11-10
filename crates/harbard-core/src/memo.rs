//! Answers of pure text functions, kept across reads.
//!
//! Every read folds the same messages again, and most of them have not
//! changed since the last read. A function whose answer depends only on its
//! text (and an optional label) can keep that answer: the next read finds it
//! by the text instead of working it out again. The whole text is compared
//! on a hit, never only its hash.

use std::collections::HashMap;
use std::hash::{BuildHasher, RandomState};
use std::sync::Mutex;

/// Above this many answers the table starts over: a read holds a few
/// hundred messages, so this only bounds a pathological history.
const MAX_ENTRIES: usize = 16_384;

struct Entry<V> {
    text: Box<str>,
    label: Option<Box<str>>,
    value: V,
}

/// Answers by `(text, label)`.
pub(crate) struct Memo<V> {
    hasher: RandomState,
    table: Mutex<HashMap<u64, Vec<Entry<V>>>>,
}

impl<V: Clone> Memo<V> {
    pub fn new() -> Self {
        Memo {
            hasher: RandomState::new(),
            table: Mutex::new(HashMap::new()),
        }
    }

    /// `f()`, or the answer it gave for the same text and label.
    pub fn get(&self, text: &str, label: Option<&str>, f: impl FnOnce() -> V) -> V {
        let key = self.hasher.hash_one((text, label));
        let same = |e: &Entry<V>| &*e.text == text && e.label.as_deref() == label;
        if let Some(v) = self
            .table
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .and_then(|bucket| bucket.iter().find(|e| same(e)))
        {
            return v.value.clone();
        }
        let value = f();
        let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
        if table.len() >= MAX_ENTRIES {
            table.clear();
        }
        let bucket = table.entry(key).or_default();
        if !bucket.iter().any(same) {
            bucket.push(Entry {
                text: text.into(),
                label: label.map(Into::into),
                value: value.clone(),
            });
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn the_same_text_and_label_are_worked_out_once() {
        let memo: Memo<usize> = Memo::new();
        let calls = Cell::new(0);
        let len = |t: &str| {
            calls.set(calls.get() + 1);
            t.len()
        };
        assert_eq!(memo.get("abc", None, || len("abc")), 3);
        assert_eq!(memo.get("abc", None, || len("abc")), 3);
        assert_eq!(calls.get(), 1);
        // Another label or another text is another answer.
        assert_eq!(memo.get("abc", Some("DONE"), || len("abcd")), 4);
        assert_eq!(memo.get("abcde", None, || len("abcde")), 5);
        assert_eq!(calls.get(), 3);
        assert_eq!(memo.get("abc", Some("DONE"), || len("x")), 4);
    }
}
