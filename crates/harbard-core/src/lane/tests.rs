use super::*;
use crate::test_util::TempHome;
use std::collections::BTreeSet;

const REPO: &str = "/work/northwind";
const LANE: &str = "migrations";
const MINUTE: u64 = 60_000;

fn request<'a>(session: &'a str, purpose: &'a str) -> LaneRequest<'a> {
    LaneRequest {
        session,
        purpose,
        ttl_secs: 600,
        release_on: &[],
    }
}

struct Settled(BTreeSet<u64>);

impl PrFacts for Settled {
    fn settled(&self, _repo: &str, number: u64) -> bool {
        self.0.contains(&number)
    }
}

#[test]
fn first_taker_owns_and_later_takers_queue_in_arrival_order() {
    let home = TempHome::new();
    let lanes = LaneStore::at(home.path());
    let first = lanes
        .take(REPO, LANE, &request("a", "0234"), &NoPrFacts, 0)
        .unwrap();
    assert_eq!(first.outcome, TakeOutcome::Acquired);
    let second = lanes
        .take(REPO, LANE, &request("b", "0235"), &NoPrFacts, 1)
        .unwrap();
    assert_eq!(second.outcome, TakeOutcome::Queued { position: 1 });
    let third = lanes
        .take(REPO, LANE, &request("c", "0236"), &NoPrFacts, 2)
        .unwrap();
    assert_eq!(third.outcome, TakeOutcome::Queued { position: 2 });
    let again = lanes
        .take(REPO, LANE, &request("b", "0235 again"), &NoPrFacts, 3)
        .unwrap();
    assert_eq!(again.outcome, TakeOutcome::Queued { position: 1 });
    assert_eq!(again.record.queue.len(), 2);
}

#[test]
fn the_owner_taking_again_renews_instead_of_queueing() {
    let home = TempHome::new();
    let lanes = LaneStore::at(home.path());
    lanes
        .take(REPO, LANE, &request("a", "0234"), &NoPrFacts, 0)
        .unwrap();
    let again = lanes
        .take(REPO, LANE, &request("a", "0234"), &NoPrFacts, 5 * MINUTE)
        .unwrap();
    assert_eq!(again.outcome, TakeOutcome::Held);
    let owner = again.record.owner.unwrap();
    assert_eq!(owner.taken_at_ms, 0);
    assert_eq!(owner.expires_at_ms, 15 * MINUTE);
}

#[test]
fn release_hands_the_lane_to_the_head_of_the_queue() {
    let home = TempHome::new();
    let lanes = LaneStore::at(home.path());
    lanes
        .take(REPO, LANE, &request("a", "x"), &NoPrFacts, 0)
        .unwrap();
    lanes
        .take(REPO, LANE, &request("b", "y"), &NoPrFacts, 1)
        .unwrap();
    lanes
        .take(REPO, LANE, &request("c", "z"), &NoPrFacts, 2)
        .unwrap();
    let released = lanes.release(REPO, LANE, "a", &NoPrFacts, 3).unwrap();
    assert_eq!(released.outcome, ReleaseOutcome::Released);
    assert_eq!(
        released.handovers,
        vec![Handover {
            repo: REPO.into(),
            lane: LANE.into(),
            reason: HandoverReason::Released,
            from: Some("a".into()),
            to: Some("b".into()),
            prs: vec![],
        }]
    );
    let record = released.record.unwrap();
    let owner = record.owner.unwrap();
    assert_eq!(owner.session, "b");
    assert_eq!(owner.taken_at_ms, 3, "the new turn's clock starts now");
    assert_eq!(record.queue.len(), 1);
    assert_eq!(record.queue[0].session, "c");
}

#[test]
fn a_waiter_can_leave_the_queue_and_a_stranger_releases_nothing() {
    let home = TempHome::new();
    let lanes = LaneStore::at(home.path());
    lanes
        .take(REPO, LANE, &request("a", "x"), &NoPrFacts, 0)
        .unwrap();
    lanes
        .take(REPO, LANE, &request("b", "y"), &NoPrFacts, 1)
        .unwrap();
    let left = lanes.release(REPO, LANE, "b", &NoPrFacts, 2).unwrap();
    assert_eq!(left.outcome, ReleaseOutcome::LeftQueue);
    let stranger = lanes.release(REPO, LANE, "z", &NoPrFacts, 3).unwrap();
    assert_eq!(stranger.outcome, ReleaseOutcome::NotHeld);
    assert!(stranger.record.unwrap().owned_by("a"));
}

#[test]
fn releasing_the_last_holder_leaves_no_file_behind() {
    let home = TempHome::new();
    let lanes = LaneStore::at(home.path());
    lanes
        .take(REPO, LANE, &request("a", "x"), &NoPrFacts, 0)
        .unwrap();
    assert_eq!(lanes.list(Some(REPO)).len(), 1);
    lanes.release(REPO, LANE, "a", &NoPrFacts, 1).unwrap();
    assert!(lanes.list(None).is_empty());
    let (view, _) = lanes.show(REPO, LANE, &NoPrFacts, 2).unwrap();
    assert!(view.is_none());
}

#[test]
fn an_expired_owner_passes_the_lane_to_the_next_waiter() {
    let home = TempHome::new();
    let lanes = LaneStore::at(home.path());
    lanes
        .take(REPO, LANE, &request("a", "x"), &NoPrFacts, 0)
        .unwrap();
    lanes
        .take(REPO, LANE, &request("b", "y"), &NoPrFacts, 1)
        .unwrap();
    let (before, handovers) = lanes.show(REPO, LANE, &NoPrFacts, 10 * MINUTE - 1).unwrap();
    assert!(handovers.is_empty());
    assert!(before.unwrap().owned_by("a"));
    let (after, handovers) = lanes.show(REPO, LANE, &NoPrFacts, 10 * MINUTE).unwrap();
    assert_eq!(handovers.len(), 1);
    assert_eq!(handovers[0].reason, HandoverReason::Expired);
    assert_eq!(handovers[0].from.as_deref(), Some("a"));
    assert_eq!(handovers[0].to.as_deref(), Some("b"));
    assert!(after.unwrap().owned_by("b"));
    let (_, again) = lanes.show(REPO, LANE, &NoPrFacts, 10 * MINUTE + 1).unwrap();
    assert!(again.is_empty(), "a handover is reported exactly once");
}

#[test]
fn an_expired_owner_with_no_waiters_frees_the_lane() {
    let home = TempHome::new();
    let lanes = LaneStore::at(home.path());
    lanes
        .take(REPO, LANE, &request("a", "x"), &NoPrFacts, 0)
        .unwrap();
    let handovers = lanes.sweep(&NoPrFacts, 11 * MINUTE);
    assert_eq!(handovers.len(), 1);
    assert_eq!(handovers[0].to, None);
    assert!(lanes.list(None).is_empty());
}

#[test]
fn heartbeat_restarts_the_owner_ttl_and_refuses_anyone_else() {
    let home = TempHome::new();
    let lanes = LaneStore::at(home.path());
    lanes
        .take(REPO, LANE, &request("a", "x"), &NoPrFacts, 0)
        .unwrap();
    let beat = lanes
        .heartbeat(REPO, LANE, "a", &NoPrFacts, 9 * MINUTE)
        .unwrap();
    assert_eq!(beat.outcome, HeartbeatOutcome::Renewed);
    assert_eq!(
        beat.record.unwrap().owner.unwrap().expires_at_ms,
        19 * MINUTE
    );
    let (still, handovers) = lanes.show(REPO, LANE, &NoPrFacts, 15 * MINUTE).unwrap();
    assert!(handovers.is_empty());
    assert!(still.unwrap().owned_by("a"));
    let other = lanes
        .heartbeat(REPO, LANE, "b", &NoPrFacts, 15 * MINUTE)
        .unwrap();
    assert_eq!(other.outcome, HeartbeatOutcome::NotOwner);
}

#[test]
fn a_late_heartbeat_learns_the_turn_already_passed() {
    let home = TempHome::new();
    let lanes = LaneStore::at(home.path());
    lanes
        .take(REPO, LANE, &request("a", "x"), &NoPrFacts, 0)
        .unwrap();
    lanes
        .take(REPO, LANE, &request("b", "y"), &NoPrFacts, 1)
        .unwrap();
    let late = lanes
        .heartbeat(REPO, LANE, "a", &NoPrFacts, 20 * MINUTE)
        .unwrap();
    assert_eq!(late.outcome, HeartbeatOutcome::NotOwner);
    assert_eq!(late.handovers.len(), 1);
    assert!(late.record.unwrap().owned_by("b"));
}

#[test]
fn a_waiter_promoted_by_expiry_sees_its_take_as_an_acquisition() {
    let home = TempHome::new();
    let lanes = LaneStore::at(home.path());
    lanes
        .take(REPO, LANE, &request("a", "x"), &NoPrFacts, 0)
        .unwrap();
    lanes
        .take(REPO, LANE, &request("b", "y"), &NoPrFacts, 1)
        .unwrap();
    let take = lanes
        .take(REPO, LANE, &request("b", "y"), &NoPrFacts, 11 * MINUTE)
        .unwrap();
    assert_eq!(take.outcome, TakeOutcome::Acquired);
}

// A promoted waiter may have ended its turn: the ttl must wait for it to wake.
#[test]
fn a_promoted_waiter_starts_its_ttl_at_its_first_sign_of_life() {
    let home = TempHome::new();
    let lanes = LaneStore::at(home.path());
    lanes
        .take(REPO, LANE, &request("a", "x"), &NoPrFacts, 0)
        .unwrap();
    lanes
        .take(REPO, LANE, &request("b", "y"), &NoPrFacts, 1)
        .unwrap();
    lanes.release(REPO, LANE, "a", &NoPrFacts, MINUTE).unwrap();
    let (view, _) = lanes.show(REPO, LANE, &NoPrFacts, MINUTE).unwrap();
    let owner = view.unwrap().owner.unwrap();
    assert_eq!(owner.session, "b");
    assert_eq!(owner.claim_by_ms, Some(16 * MINUTE));
    assert_eq!(
        owner.expires_at_ms,
        16 * MINUTE,
        "grace, not the 10 min ttl"
    );

    // Asleep past its own 10-minute ttl: the turn is still waiting for it.
    let (view, handovers) = lanes.show(REPO, LANE, &NoPrFacts, 12 * MINUTE).unwrap();
    assert!(handovers.is_empty());
    assert!(view.unwrap().owned_by("b"));

    assert!(lanes.claim("b", &NoPrFacts, 12 * MINUTE).is_empty());
    let (view, _) = lanes.show(REPO, LANE, &NoPrFacts, 12 * MINUTE).unwrap();
    let owner = view.unwrap().owner.unwrap();
    assert_eq!(owner.claim_by_ms, None);
    assert_eq!(
        owner.expires_at_ms,
        22 * MINUTE,
        "the whole ttl from waking"
    );

    // A second sign of life is not a heartbeat: only the first one counts.
    lanes.claim("b", &NoPrFacts, 20 * MINUTE);
    let (_, handovers) = lanes.show(REPO, LANE, &NoPrFacts, 22 * MINUTE).unwrap();
    assert_eq!(handovers[0].reason, HandoverReason::Expired);
}

// The same night another session held two lanes that both expired before it
// was next prompted. A waiter that never wakes must not block the queue.
#[test]
fn an_unclaimed_promotion_passes_on_after_the_grace_period() {
    let home = TempHome::new();
    let lanes = LaneStore::at(home.path());
    for (session, at) in [("a", 0), ("b", 1), ("c", 2)] {
        lanes
            .take(REPO, LANE, &request(session, "x"), &NoPrFacts, at)
            .unwrap();
    }
    lanes.release(REPO, LANE, "a", &NoPrFacts, 3).unwrap();
    let grace_ends = 3 + PROMOTION_GRACE_SECS * 1000;
    let (_, before) = lanes.show(REPO, LANE, &NoPrFacts, grace_ends - 1).unwrap();
    assert!(before.is_empty());
    let (view, handovers) = lanes.show(REPO, LANE, &NoPrFacts, grace_ends).unwrap();
    assert_eq!(handovers.len(), 1);
    assert_eq!(handovers[0].reason, HandoverReason::Unclaimed);
    assert_eq!(handovers[0].from.as_deref(), Some("b"));
    assert_eq!(handovers[0].to.as_deref(), Some("c"));
    assert!(view.unwrap().owned_by("c"));
    // Coming back too late revives nothing.
    lanes.claim("b", &NoPrFacts, grace_ends + 1);
    let (view, _) = lanes.show(REPO, LANE, &NoPrFacts, grace_ends + 1).unwrap();
    assert!(view.unwrap().owned_by("c"));
}

#[test]
fn a_waiter_whose_own_call_promoted_it_starts_its_ttl_at_once() {
    let home = TempHome::new();
    let lanes = LaneStore::at(home.path());
    lanes
        .take(REPO, LANE, &request("a", "x"), &NoPrFacts, 0)
        .unwrap();
    lanes
        .take(REPO, LANE, &request("b", "y"), &NoPrFacts, 1)
        .unwrap();
    let take = lanes
        .take(REPO, LANE, &request("b", "y"), &NoPrFacts, 11 * MINUTE)
        .unwrap();
    let owner = take.record.owner.unwrap();
    assert_eq!(owner.claim_by_ms, None);
    assert_eq!(owner.expires_at_ms, 21 * MINUTE);
}

#[test]
fn release_on_fires_only_when_every_listed_pr_has_settled() {
    let home = TempHome::new();
    let lanes = LaneStore::at(home.path());
    let tied = LaneRequest {
        release_on: &[2258, 2263],
        ..request("a", "0234-0236")
    };
    lanes.take(REPO, LANE, &tied, &NoPrFacts, 0).unwrap();
    lanes
        .take(REPO, LANE, &request("b", "0237"), &NoPrFacts, 1)
        .unwrap();
    let one = Settled([2258].into());
    assert!(lanes.sweep(&one, 2).is_empty());
    let both = Settled([2258, 2263].into());
    let handovers = lanes.sweep(&both, 3);
    assert_eq!(handovers.len(), 1);
    assert_eq!(handovers[0].reason, HandoverReason::PrsSettled);
    assert_eq!(handovers[0].prs, vec![2258, 2263]);
    assert_eq!(handovers[0].to.as_deref(), Some("b"));
}

#[test]
fn an_ended_session_releases_its_lanes_and_leaves_every_queue() {
    let home = TempHome::new();
    let lanes = LaneStore::at(home.path());
    lanes
        .take(REPO, LANE, &request("a", "x"), &NoPrFacts, 0)
        .unwrap();
    lanes
        .take(REPO, LANE, &request("b", "y"), &NoPrFacts, 1)
        .unwrap();
    lanes
        .take(REPO, "dev-stack", &request("c", "z"), &NoPrFacts, 2)
        .unwrap();
    lanes
        .take(REPO, "dev-stack", &request("a", "w"), &NoPrFacts, 3)
        .unwrap();
    let handovers = lanes.release_session("a", &NoPrFacts, 4);
    assert_eq!(handovers.len(), 1);
    assert_eq!(handovers[0].reason, HandoverReason::SessionEnded);
    assert_eq!(handovers[0].to.as_deref(), Some("b"));
    let stack = lanes
        .list(Some(REPO))
        .into_iter()
        .find(|record| record.lane == "dev-stack")
        .unwrap();
    assert!(stack.queue.is_empty());
}

#[test]
fn lanes_are_scoped_to_their_repository() {
    let home = TempHome::new();
    let lanes = LaneStore::at(home.path());
    lanes
        .take(REPO, LANE, &request("a", "x"), &NoPrFacts, 0)
        .unwrap();
    let elsewhere = lanes
        .take("/work/other", LANE, &request("b", "y"), &NoPrFacts, 1)
        .unwrap();
    assert_eq!(elsewhere.outcome, TakeOutcome::Acquired);
    assert_eq!(lanes.list(Some(REPO)).len(), 1);
    assert_eq!(lanes.list(None).len(), 2);
}

#[test]
fn invalid_names_and_requests_are_refused() {
    let home = TempHome::new();
    let lanes = LaneStore::at(home.path());
    for name in ["", "Migrations", "-x", "a b", &"x".repeat(65)] {
        assert!(matches!(
            lanes.take(REPO, name, &request("a", "x"), &NoPrFacts, 0),
            Err(LaneError::InvalidLane(_))
        ));
    }
    assert!(matches!(
        lanes.take(REPO, LANE, &request("a", "  "), &NoPrFacts, 0),
        Err(LaneError::InvalidRequest(_))
    ));
    let forever = LaneRequest {
        ttl_secs: MAX_LANE_TTL_SECS + 1,
        ..request("a", "x")
    };
    assert!(matches!(
        lanes.take(REPO, LANE, &forever, &NoPrFacts, 0),
        Err(LaneError::InvalidRequest(_))
    ));
}

#[test]
fn corrupt_state_is_refused_rather_than_overwritten() {
    let home = TempHome::new();
    let lanes = LaneStore::at(home.path());
    lanes
        .take(REPO, LANE, &request("a", "x"), &NoPrFacts, 0)
        .unwrap();
    let path = lanes.root().join(format!("{}.json", lane_hash(REPO, LANE)));
    std::fs::write(&path, b"{not json").unwrap();
    assert!(matches!(
        lanes.take(REPO, LANE, &request("b", "y"), &NoPrFacts, 1),
        Err(LaneError::CorruptState)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), b"{not json");
}
