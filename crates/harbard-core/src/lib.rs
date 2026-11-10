//! harbard-core: the cross-harness session bus, storage and read model (F0).
//!
//! Boring storage (append-only JSONL, skip-and-count), evidence-based presence
//! (liveness = fresh signal, never `hello`/`bye` declarations), and an
//! injection-hardened `brief` digest. See `SPEC.md`.

pub mod activity;
pub mod agent_feedback;
pub mod alias;
pub mod brief;
pub mod checkout_history;
pub mod clock;
pub mod coded;
pub mod cursor;
pub mod dependency;
pub mod device;
pub mod eval;
pub mod event;
pub mod feed;
pub mod feedback;
pub mod fleet;
pub mod fts;
pub mod harness_trace;
pub mod history;
pub mod hold;
pub mod human;
pub mod id;
pub mod lane;
pub mod learning;
pub mod lease;
pub mod lifecycle;
pub mod live;
pub mod log_watch;
pub mod membership;
pub mod naming;
pub mod note_trace;
pub mod old_spelling;
pub mod operator;
pub mod oplog;
pub mod pairing;
pub mod pr_watch;
pub mod process_start;
pub mod profile;
pub mod progress;
pub mod protocol;
pub mod prune;
pub mod query;
pub mod reach;
pub mod readable;
pub mod redact;
pub mod report;
pub mod resolution;
pub mod retro;
pub mod rooms;
pub mod rules;
pub mod safe_fs;
pub mod seal;
pub mod shipments;
pub mod sluice;
pub mod status;
pub mod store;
mod memo;
pub mod stamped;
pub mod team;
mod team_scan;
pub mod theme;
pub mod thread;
pub mod topology;
pub mod transcript_archive;
pub mod wake;
pub mod workshop;
pub mod workspace;
pub mod workspaces;
pub mod write_class;

#[cfg(test)]
mod test_util;

pub use device::{
    Access, DeviceCard, DeviceRegistry, DeviceRole, HARNESS_DEVICE, LocalDevice, card_summary,
    device_host, stamped_host,
};
pub use event::NoteKind;
pub use event::{
    Event, EventError, EventKind, HARNESS_HUMAN, MAX_LINE_BYTES, MAX_TEXT_BYTES, Reader, Recipient,
    RecipientKind, Role, SCHEMA_VERSION, is_event_id,
};
pub use feed::{FeedItem, FeedPage, Place};
pub use fleet::{
    CheckoutCard, Counts, FleetInputs, FleetView, PendingItem, Presence, RoomCard, SessionCard,
};
pub use fts::{FtsStats, RebuildStats, SearchHit, default_limit as fts_default_limit};
pub use lease::{
    DEFAULT_LEASE_TTL_SECS, LeaseCheck, LeaseClaim, LeaseClock, LeaseEnsure, LeaseError, LeaseKind,
    LeaseMetadata, LeaseRecord, LeaseRelease, LeaseResource, LeaseStats, LeaseStore, PidChecker,
    SystemLeaseClock, SystemPidChecker, WriterLeaseView, same_writer_lineage,
};
pub use lifecycle::{Hazard, Policy as LifecyclePolicy, Unreadable};
pub use membership::{
    AnchorKind, ArchiveCandidate, BulkArchiveItem, BulkArchiveStatus, CHANNEL_UNUSED_GRACE_MS,
    ChannelHygiene, ChannelHygieneReport, ChannelMeta, IneligibleReason, Member, Memberships,
    RoomVisibility, archive_bulk, archive_candidates, channel_unused_resolution_id,
    classify_channel, delete_archived, has_members, hygiene_report, mark_read, mark_room_read,
    touch_note_activity, update_room, visible_rooms_for,
};
pub use operator::{OperatorProfile, sanitize_display_name};
pub use protocol::{
    Capability, Correction, PROTOCOL_VERSION, ProtocolDepth, Receipt, Rule, catalog, render,
    sections,
};
pub use query::{DoctorReport, InboxResult, Liveness, SessionView};
pub use resolution::{RESOLUTION_TTL_MS, ReceiptStage, Resolution, ResolutionStore};
pub use sluice::{Delivery, Receipt as SluiceReceipt, Verdict};
pub use store::{
    DEFAULT_RETENTION_DAYS, HistoryError, HistoryFilter, HistoryPage, ReadPosition, ReadResult,
    Store, Window,
};
pub use topology::{
    IdentityConfidence, distinct_writer_domains, infer_identity_confidence, is_rival_writer,
    is_writer_domain_member, looks_like_host_pid_session, writer_domain_root,
};
pub use write_class::{WriteClass, classify_shell, classify_tool};

/// The machine hostname, best-effort, without shelling out.
pub fn hostname() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}

/// Lowercase hex of a digest. sha2 0.11 dropped `LowerHex` on its output.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

#[cfg(test)]
mod hex_tests {
    use sha2::{Digest, Sha256};

    /// Stored hashes (lease checkouts, delivery ids) must keep the spelling
    /// sha2 0.10 printed with `{:x}`: the FIPS 180-2 test vector for "abc".
    #[test]
    fn a_digest_reads_as_the_same_lowercase_hex_as_before() {
        assert_eq!(
            super::hex(&Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
