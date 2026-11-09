use super::*;
use crate::test_util::TempHome;
use std::sync::{Arc, Barrier};

#[derive(Clone, Copy)]
struct FakePid(bool);

impl PidChecker for FakePid {
    fn is_alive(&self, _pid: u32) -> bool {
        self.0
    }
}

/// Every pid is alive except one — the orphaned-supervisor case.
#[derive(Clone, Copy)]
struct DeadPid(u32);

impl PidChecker for DeadPid {
    fn is_alive(&self, pid: u32) -> bool {
        pid != self.0
    }
}

fn resource() -> LeaseResource {
    LeaseResource::writer("host", "/repo/.git", "/repo/worktree").unwrap()
}

fn metadata(session: &str, pid: Option<u32>) -> LeaseMetadata {
    LeaseMetadata {
        session: session.to_string(),
        harness: "codex".to_string(),
        pid,
        parent: None,
        repository_id: Some("/repo/.git".to_string()),
        branch: Some("codex/lease-foundation".to_string()),
        head_sha: Some("0123456789012345678901234567890123456789".to_string()),
    }
}

fn metadata_child(session: &str, parent: &str, pid: Option<u32>) -> LeaseMetadata {
    let mut m = metadata(session, pid);
    m.parent = Some(parent.to_string());
    m
}

/// The bug this guards: a session that already held the checkout asked again
/// from a background build — same session, different supervisor pid — and got
/// `conflict`, which every agent read as "a peer owns this tree". It then
/// stood down, warned the innocent peer on the bus, and asked the operator
/// what to do about a lease that was its own.
#[test]
fn the_holder_reclaiming_renews_in_place_and_a_rival_still_conflicts() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());

    let first = store
        .claim(
            &resource(),
            &metadata("mine", Some(63030)),
            900,
            1_000,
            &FakePid(true),
        )
        .unwrap();
    let LeaseClaim::Acquired(held) = first else {
        panic!("the first claim on a free resource must acquire");
    };

    // Same session, pid the tool shell could not resolve back to the supervisor.
    let again = store
        .claim(
            &resource(),
            &metadata("mine", None),
            900,
            2_000,
            &FakePid(true),
        )
        .unwrap();
    match again {
        LeaseClaim::Renewed(lease) => {
            assert_eq!(lease.claim_id, held.claim_id, "the receipt must survive");
            assert_eq!(lease.claimed_at_ms, held.claimed_at_ms);
            assert_eq!(lease.renewed_at_ms, 2_000, "renewal moves the clock");
            assert_eq!(lease.expires_at_ms, 2_000 + 900_000);
        }
        other => panic!("the holder must renew, got {other:?}"),
    }

    // A repeat claim carrying a different ttl renews on the RECORDED window,
    // never its own: the holder's 900s must not shrink to 60s because some
    // later call passed a different flag. Writing one without the other would
    // also break `read_record`'s expires == renewed + ttl and poison the
    // resource for every reader.
    let retimed = store
        .claim(
            &resource(),
            &metadata("mine", None),
            60,
            2_500,
            &FakePid(true),
        )
        .unwrap();
    match retimed {
        LeaseClaim::Renewed(lease) => {
            assert_eq!(lease.ttl_secs, 900, "the holder's window survives");
            assert_eq!(lease.expires_at_ms, 2_500 + 900_000);
        }
        other => panic!("the holder must renew, got {other:?}"),
    }
    assert!(
        store
            .check(&resource(), None, false, 2_600, &FakePid(true))
            .is_ok(),
        "a renewed record must still read back as valid"
    );

    // A child of the holder shares the tree: a herd member is not a rival.
    let child = store
        .claim(
            &resource(),
            &metadata_child("herd-1", "mine", None),
            900,
            3_000,
            &FakePid(true),
        )
        .unwrap();
    assert!(matches!(child, LeaseClaim::Renewed(_)), "got {child:?}");

    // An unrelated session is still refused — the invariant that pays for all this.
    let rival = store
        .claim(
            &resource(),
            &metadata("theirs", Some(99)),
            900,
            4_000,
            &FakePid(true),
        )
        .unwrap();
    match rival {
        LeaseClaim::Conflict(lease) => assert_eq!(lease.session, "mine"),
        other => panic!("a rival must be refused, got {other:?}"),
    }
}

#[test]
fn one_of_one_hundred_concurrent_claims_wins() {
    let home = TempHome::new();
    let store = Arc::new(LeaseStore::at(home.path()));
    let barrier = Arc::new(Barrier::new(100));
    let mut handles = Vec::new();
    for index in 0..100 {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            store
                .claim(
                    &resource(),
                    &metadata(&format!("session-{index}"), None),
                    60,
                    1_000_000,
                    &FakePid(true),
                )
                .unwrap()
        }));
    }
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, LeaseClaim::Acquired(_)))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, LeaseClaim::Conflict(_)))
            .count(),
        99
    );
}

#[test]
fn different_worktrees_can_hold_distinct_writer_leases() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let first = resource();
    let second = LeaseResource::writer("host", "/repo/.git", "/repo/other").unwrap();
    assert!(matches!(
        store
            .claim(&first, &metadata("one", None), 60, 1_000, &FakePid(true))
            .unwrap(),
        LeaseClaim::Acquired(_)
    ));
    assert!(matches!(
        store
            .claim(&second, &metadata("two", None), 60, 1_000, &FakePid(true))
            .unwrap(),
        LeaseClaim::Acquired(_)
    ));
}

#[test]
fn ttl_and_dead_pid_both_allow_reclaim() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let resource = resource();
    let first = store
        .claim(
            &resource,
            &metadata("one", Some(42)),
            1,
            1_000,
            &FakePid(true),
        )
        .unwrap();
    let first_id = match first {
        LeaseClaim::Acquired(lease) => lease.claim_id,
        LeaseClaim::Renewed(_) | LeaseClaim::Conflict(_) => unreachable!(),
    };

    // Even a reused/live PID cannot preserve an expired lease.
    let second = store
        .claim(
            &resource,
            &metadata("two", Some(42)),
            60,
            2_000,
            &FakePid(true),
        )
        .unwrap();
    let second_id = match second {
        LeaseClaim::Acquired(lease) => lease.claim_id,
        LeaseClaim::Renewed(_) | LeaseClaim::Conflict(_) => unreachable!(),
    };
    assert_ne!(first_id, second_id);

    // A dead PID also makes an otherwise fresh lease reclaimable.
    assert!(matches!(
        store
            .claim(
                &resource,
                &metadata("three", None),
                60,
                2_100,
                &FakePid(false),
            )
            .unwrap(),
        LeaseClaim::Acquired(_)
    ));
}

#[test]
fn old_release_is_fenced_after_reclaim() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let resource = resource();
    let first = store
        .claim(&resource, &metadata("one", None), 1, 1_000, &FakePid(true))
        .unwrap();
    let first_id = match first {
        LeaseClaim::Acquired(lease) => lease.claim_id,
        LeaseClaim::Renewed(_) | LeaseClaim::Conflict(_) => unreachable!(),
    };
    let second = store
        .claim(&resource, &metadata("two", None), 60, 2_000, &FakePid(true))
        .unwrap();
    let second_id = match second {
        LeaseClaim::Acquired(lease) => lease.claim_id,
        LeaseClaim::Renewed(_) | LeaseClaim::Conflict(_) => unreachable!(),
    };
    assert!(matches!(
        store.release(&resource, &first_id, 2_100).unwrap(),
        LeaseRelease::Fenced(lease) if lease.claim_id == second_id
    ));
    assert!(matches!(
        store
            .check(&resource, Some(&second_id), false, 2_100, &FakePid(true))
            .unwrap(),
        LeaseCheck::Active {
            matches_claim: Some(true),
            ..
        }
    ));
}

#[test]
fn release_is_idempotent_only_once_resource_is_free() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let resource = resource();
    let claimed = store
        .claim(&resource, &metadata("one", None), 60, 1_000, &FakePid(true))
        .unwrap();
    let claim_id = match claimed {
        LeaseClaim::Acquired(lease) => lease.claim_id,
        LeaseClaim::Renewed(_) | LeaseClaim::Conflict(_) => unreachable!(),
    };
    assert!(matches!(
        store.release(&resource, &claim_id, 1_100).unwrap(),
        LeaseRelease::Released(_)
    ));
    assert!(matches!(
        store.release(&resource, &claim_id, 1_200).unwrap(),
        LeaseRelease::Free
    ));
}

#[test]
fn writer_release_requires_claim_session_and_live_supervisor_pid() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let resource = resource();
    let owner = metadata("owner", Some(42));
    let claim_id = match store
        .claim(&resource, &owner, 60, 1_000, &FakePid(true))
        .unwrap()
    {
        LeaseClaim::Acquired(lease) => lease.claim_id,
        LeaseClaim::Renewed(_) | LeaseClaim::Conflict(_) => unreachable!(),
    };

    for requester in [metadata("rival", Some(42)), metadata("owner", Some(43))] {
        assert!(matches!(
            store
                .renew_owned(&resource, &claim_id, &requester, 1_050, &FakePid(true))
                .unwrap(),
            LeaseCheck::Active {
                matches_claim: Some(false),
                ..
            }
        ));
        assert!(matches!(
            store
                .release_owned(&resource, &claim_id, &requester, 1_100, &FakePid(true))
                .unwrap(),
            LeaseRelease::Fenced(_)
        ));
    }
    assert!(matches!(
        store
            .renew_owned(
                &resource,
                &claim_id,
                &owner,
                1_150,
                &FakePid(true)
            )
            .unwrap(),
        LeaseCheck::Active {
            lease,
            matches_claim: Some(true)
        } if lease.renewed_at_ms == 1_150
    ));
    assert!(matches!(
        store
            .release_owned(&resource, &claim_id, &owner, 1_200, &FakePid(false))
            .unwrap(),
        LeaseRelease::Fenced(_)
    ));
    assert!(matches!(
        store
            .release_owned(&resource, &claim_id, &owner, 1_300, &FakePid(true))
            .unwrap(),
        LeaseRelease::Released(_)
    ));
}

#[test]
fn session_end_can_release_target_leases_across_worktrees() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let owner = metadata("owner", Some(42));
    let first = resource();
    let second = LeaseResource::writer("host", "/repo/.git", "/repo/other").unwrap();
    for target in [&first, &second] {
        assert!(matches!(
            store
                .claim(target, &owner, 60, 1_000, &FakePid(true))
                .unwrap(),
            LeaseClaim::Acquired(_)
        ));
    }

    assert_eq!(
        store
            .release_owned_writers(&owner, 1_100, &FakePid(true))
            .unwrap(),
        2
    );
    for target in [&first, &second] {
        assert!(matches!(
            store
                .check(target, None, false, 1_200, &FakePid(true))
                .unwrap(),
            LeaseCheck::Free
        ));
    }
}

#[test]
fn corrupt_state_fails_closed_for_every_operation() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let resource = resource();
    store
        .claim(&resource, &metadata("seed", None), 60, 900, &FakePid(true))
        .unwrap();
    let paths = store.resource_paths(&resource);
    fs::write(&paths.state, b"not-json").unwrap();
    #[cfg(unix)]
    fs::set_permissions(
        &paths.state,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .unwrap();

    assert!(matches!(
        store.claim(&resource, &metadata("one", None), 60, 1_000, &FakePid(true)),
        Err(LeaseError::CorruptState)
    ));
    assert!(matches!(
        store.check(&resource, None, false, 1_000, &FakePid(true)),
        Err(LeaseError::CorruptState)
    ));
    assert!(matches!(
        store.release(&resource, "01ARZ3NDEKTSV4RRFFQ69G5FAV", 1_000),
        Err(LeaseError::CorruptState)
    ));
    assert_eq!(fs::read(&paths.state).unwrap(), b"not-json");
}

#[test]
fn matching_active_check_can_renew_but_stale_check_cannot() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let resource = resource();
    let claimed = store
        .claim(&resource, &metadata("one", None), 1, 1_000, &FakePid(true))
        .unwrap();
    let claim_id = match claimed {
        LeaseClaim::Acquired(lease) => lease.claim_id,
        LeaseClaim::Renewed(_) | LeaseClaim::Conflict(_) => unreachable!(),
    };
    let renewed = store
        .check(&resource, Some(&claim_id), true, 1_500, &FakePid(true))
        .unwrap();
    assert!(matches!(
        renewed,
        LeaseCheck::Active { lease, .. } if lease.expires_at_ms == 2_500
    ));
    let stale = store
        .check(&resource, Some(&claim_id), true, 2_500, &FakePid(true))
        .unwrap();
    assert!(matches!(stale, LeaseCheck::Stale { .. }));
}

#[test]
fn doctor_stats_count_active_stale_corrupt_and_temps() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let active = LeaseResource::service("host", "operations.northwind.localhost").unwrap();
    let stale = LeaseResource::service("host", "platform.northwind.localhost").unwrap();
    store
        .claim(&active, &metadata("one", None), 60, 1_000, &FakePid(true))
        .unwrap();
    store
        .claim(&stale, &metadata("two", None), 1, 1_000, &FakePid(true))
        .unwrap();
    let corrupt = LeaseResource::service("host", "identity.northwind.localhost").unwrap();
    let paths = store.resource_paths(&corrupt);
    ensure_private_directory(&paths.directory).unwrap();
    fs::write(&paths.state, b"{").unwrap();
    #[cfg(unix)]
    fs::set_permissions(
        &paths.state,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .unwrap();
    fs::write(paths.directory.join("orphan.0.x.tmp"), b"").unwrap();

    let stats = store.stats(2_000, &FakePid(true));
    assert_eq!(stats.total, 3);
    assert_eq!(stats.active, 1);
    assert_eq!(stats.stale, 1);
    assert_eq!(stats.corrupt, 1);
    assert_eq!(stats.inaccessible, 0);
    assert!(!stats.healthy);
    assert_eq!(stats.temp_files, 1);
}

#[test]
fn claim_garbage_collects_only_old_temps_for_its_resource() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let resource = resource();
    store
        .claim(&resource, &metadata("one", None), 60, 1, &FakePid(true))
        .unwrap();
    let paths = store.resource_paths(&resource);
    let old = paths.directory.join(format!("{}.1.old.tmp", paths.hash));
    let fresh = paths.directory.join(format!(
        "{}.{}.fresh.tmp",
        paths.hash,
        TEMP_RETENTION_MS + 1
    ));
    fs::write(&old, b"").unwrap();
    fs::write(&fresh, b"").unwrap();

    store
        .check(
            &resource,
            None,
            false,
            TEMP_RETENTION_MS + 2,
            &FakePid(true),
        )
        .unwrap();
    assert!(!old.exists());
    assert!(fresh.exists());
}

#[test]
fn dead_pid_is_rejected_before_state_is_written() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let resource = resource();
    assert!(matches!(
        store.claim(
            &resource,
            &metadata("dead", Some(42)),
            60,
            1_000,
            &FakePid(false),
        ),
        Err(LeaseError::DeadPid)
    ));
    assert!(!store.resource_paths(&resource).state.exists());
}

#[test]
fn embedded_resource_and_metadata_are_revalidated_on_read() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let resource = resource();
    let claimed = store
        .claim(&resource, &metadata("one", None), 60, 1_000, &FakePid(true))
        .unwrap();
    let lease = match claimed {
        LeaseClaim::Acquired(lease) => lease,
        LeaseClaim::Renewed(_) | LeaseClaim::Conflict(_) => unreachable!(),
    };
    let paths = store.resource_paths(&resource);

    let mut wrong_resource = lease.clone();
    wrong_resource.resource.worktree = Some("/repo/other".to_string());
    fs::write(&paths.state, serde_json::to_vec(&wrong_resource).unwrap()).unwrap();
    assert!(matches!(
        store.check(&resource, None, false, 1_100, &FakePid(true)),
        Err(LeaseError::CorruptState)
    ));

    let mut invalid_metadata = lease;
    invalid_metadata.session = "bad\nsession".to_string();
    fs::write(&paths.state, serde_json::to_vec(&invalid_metadata).unwrap()).unwrap();
    assert!(matches!(
        store.check(&resource, None, false, 1_100, &FakePid(true)),
        Err(LeaseError::CorruptState)
    ));
}

#[cfg(unix)]
#[test]
fn symlinked_lock_and_state_are_rejected_without_touching_targets() {
    use std::os::unix::fs::symlink;
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let resource = resource();
    let claimed = store
        .claim(&resource, &metadata("one", None), 60, 1_000, &FakePid(true))
        .unwrap();
    let claim_id = match claimed {
        LeaseClaim::Acquired(lease) => lease.claim_id,
        LeaseClaim::Renewed(_) | LeaseClaim::Conflict(_) => unreachable!(),
    };
    let paths = store.resource_paths(&resource);
    store.release(&resource, &claim_id, 1_100).unwrap();

    let target = home.path().join("target");
    fs::write(&target, b"unchanged").unwrap();
    symlink(&target, &paths.state).unwrap();
    assert!(
        store
            .claim(&resource, &metadata("two", None), 60, 1_200, &FakePid(true))
            .is_err()
    );
    assert_eq!(fs::read(&target).unwrap(), b"unchanged");
    fs::remove_file(&paths.state).unwrap();

    fs::remove_file(&paths.lock).unwrap();
    symlink(&target, &paths.lock).unwrap();
    assert!(
        store
            .claim(&resource, &metadata("two", None), 60, 1_200, &FakePid(true))
            .is_err()
    );
    assert_eq!(fs::read(&target).unwrap(), b"unchanged");
}

#[cfg(unix)]
#[test]
fn fifo_state_fails_fast_instead_of_blocking() {
    use nix::sys::stat::Mode;
    use nix::unistd::mkfifo;
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let resource = resource();
    let claimed = store
        .claim(&resource, &metadata("one", None), 60, 1_000, &FakePid(true))
        .unwrap();
    let claim_id = match claimed {
        LeaseClaim::Acquired(lease) => lease.claim_id,
        LeaseClaim::Renewed(_) | LeaseClaim::Conflict(_) => unreachable!(),
    };
    let paths = store.resource_paths(&resource);
    store.release(&resource, &claim_id, 1_100).unwrap();
    mkfifo(&paths.state, Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
    assert!(
        store
            .check(&resource, None, false, 1_200, &FakePid(true))
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn symlinked_lease_root_is_rejected_and_doctor_is_unhealthy() {
    use std::os::unix::fs::symlink;
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let resource = LeaseResource::service("host", "root-symlink-service").unwrap();
    let claimed = store
        .claim(&resource, &metadata("one", None), 60, 1_000, &FakePid(true))
        .unwrap();
    let claim_id = match claimed {
        LeaseClaim::Acquired(lease) => lease.claim_id,
        LeaseClaim::Renewed(_) | LeaseClaim::Conflict(_) => unreachable!(),
    };
    let paths = store.resource_paths(&resource);
    store.release(&resource, &claim_id, 1_100).unwrap();
    fs::remove_file(&paths.lock).unwrap();
    fs::remove_dir(&paths.directory).unwrap();
    fs::remove_dir(store.root()).unwrap();

    let target = home.path().join("redirected-root");
    fs::create_dir(&target).unwrap();
    symlink(&target, store.root()).unwrap();
    assert!(
        store
            .claim(&resource, &metadata("two", None), 60, 1_200, &FakePid(true))
            .is_err()
    );
    let stats = store.stats(1_200, &FakePid(true));
    assert!(!stats.healthy);
    assert!(stats.inaccessible > 0);
}

#[cfg(unix)]
#[test]
fn lease_directories_and_files_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let resource = resource();
    store
        .claim(&resource, &metadata("one", None), 60, 1_000, &FakePid(true))
        .unwrap();
    let paths = store.resource_paths(&resource);
    assert_eq!(
        fs::metadata(store.root()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&paths.directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&paths.state).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(&paths.lock).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn empty_lease_state_is_free_not_corrupt() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let empty = LeaseResource::service("host", "empty.northwind.localhost").unwrap();
    store
        .claim(&empty, &metadata("one", None), 60, 1_000, &FakePid(true))
        .unwrap();
    let paths = store.resource_paths(&empty);
    fs::write(&paths.state, b"").unwrap();
    #[cfg(unix)]
    fs::set_permissions(
        &paths.state,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .unwrap();

    let stats = store.stats(2_000, &FakePid(true));
    assert_eq!(stats.corrupt, 0, "{stats:?}");
    assert_eq!(stats.total, 0, "{stats:?}");
    assert!(stats.healthy, "{stats:?}");
}

#[test]
fn legacy_stefna_hash_filenames_migrate_and_are_not_corrupt() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let resource = LeaseResource::service("host", "legacy.northwind.localhost").unwrap();
    store
        .claim(&resource, &metadata("one", None), 60, 1_000, &FakePid(true))
        .unwrap();
    let paths = store.resource_paths(&resource);
    let bytes = fs::read(&paths.state).unwrap();
    let legacy = resource.legacy_key_hash();
    let legacy_state = paths.directory.join(format!("{legacy}.json"));
    fs::rename(&paths.state, &legacy_state).unwrap();
    let _ = fs::rename(&paths.lock, paths.directory.join(format!("{legacy}.lock")));

    let stats = store.stats(2_000, &FakePid(true));
    assert_eq!(stats.corrupt, 0, "{stats:?}");
    assert!(paths.state.exists(), "migrated onto current hash");
    assert!(!legacy_state.exists(), "legacy name removed");
    assert_eq!(fs::read(&paths.state).unwrap(), bytes);
}

#[test]
fn ensure_acquires_then_holds_for_same_session_and_blocks_rival() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let resource = resource();
    let first = store
        .ensure(
            &resource,
            &metadata("owner", Some(7)),
            60,
            1_000,
            &FakePid(true),
        )
        .unwrap();
    let LeaseEnsure::Acquired(acquired) = first else {
        panic!("expected acquired");
    };
    let held = store
        .ensure(
            &resource,
            &metadata("owner", Some(7)),
            60,
            1_500,
            &FakePid(true),
        )
        .unwrap();
    let LeaseEnsure::Held(same) = held else {
        panic!("expected held");
    };
    assert_eq!(same.claim_id, acquired.claim_id);
    assert!(same.expires_at_ms > acquired.expires_at_ms);
    assert!(matches!(
        store
            .ensure(
                &resource,
                &metadata("rival", Some(8)),
                60,
                1_600,
                &FakePid(true),
            )
            .unwrap(),
        LeaseEnsure::Conflict(_)
    ));
}

#[test]
fn ensure_parent_child_lineage_shares_without_same_pid() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let resource = resource();
    let first = store
        .ensure(
            &resource,
            &metadata("parent", Some(11)),
            60,
            1_000,
            &FakePid(true),
        )
        .unwrap();
    let LeaseEnsure::Acquired(acquired) = first else {
        panic!("expected acquired");
    };
    // Different pid, but claimant names parent as lineage — M5c share.
    let child = store
        .ensure(
            &resource,
            &metadata_child("child-herd", "parent", Some(99)),
            60,
            1_500,
            &FakePid(true),
        )
        .unwrap();
    let LeaseEnsure::Held(shared) = child else {
        panic!("expected held for parent lineage, got {child:?}");
    };
    assert_eq!(shared.claim_id, acquired.claim_id);
    assert_eq!(shared.session, "parent");
    // Unrelated session with different pid still conflicts.
    assert!(matches!(
        store
            .ensure(
                &resource,
                &metadata("foreign", Some(77)),
                60,
                1_600,
                &FakePid(true),
            )
            .unwrap(),
        LeaseEnsure::Conflict(_)
    ));
}

#[test]
fn ensure_same_supervisor_pid_shares_lease_across_sessions() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let resource = resource();
    let first = store
        .ensure(
            &resource,
            &metadata("parent", Some(42)),
            60,
            1_000,
            &FakePid(true),
        )
        .unwrap();
    let LeaseEnsure::Acquired(acquired) = first else {
        panic!("expected acquired");
    };
    // Herd / Task child: different session id, same supervisor pid.
    let herd = store
        .ensure(
            &resource,
            &metadata("child-task", Some(42)),
            60,
            1_500,
            &FakePid(true),
        )
        .unwrap();
    let LeaseEnsure::Held(shared) = herd else {
        panic!("expected held for same supervisor pid, got {herd:?}");
    };
    assert_eq!(shared.claim_id, acquired.claim_id);
    assert_eq!(shared.session, "parent"); // owner stays the original claimer
    // Different pid still conflicts.
    assert!(matches!(
        store
            .ensure(
                &resource,
                &metadata("foreign", Some(99)),
                60,
                1_600,
                &FakePid(true),
            )
            .unwrap(),
        LeaseEnsure::Conflict(_)
    ));
}

#[test]
fn release_session_is_idempotent_and_dead_pid_allows_reclaim() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let resource = resource();
    store
        .ensure(
            &resource,
            &metadata("owner", Some(9)),
            60,
            1_000,
            &FakePid(true),
        )
        .unwrap();
    assert!(matches!(
        store.release_session(&resource, "owner", 1_100).unwrap(),
        LeaseRelease::Released(_)
    ));
    assert!(matches!(
        store.release_session(&resource, "owner", 1_200).unwrap(),
        LeaseRelease::Free
    ));
    store
        .ensure(
            &resource,
            &metadata("owner", Some(10)),
            60,
            1_300,
            &FakePid(true),
        )
        .unwrap();
    // Dead PID makes the prior lease inactive; a new session without a pid
    // (or with a live pid) can reclaim without waiting for TTL.
    assert!(matches!(
        store
            .ensure(
                &resource,
                &metadata("other", None),
                60,
                1_400,
                &FakePid(false),
            )
            .unwrap(),
        LeaseEnsure::Acquired(_)
    ));
}

/// What the console reads to answer "who owns this checkout". Three things
/// must hold or the board lies: an expired or orphaned holder is NOT an owner,
/// a host-global service lease is not a checkout owner at all, and the view
/// never carries the holder's pid — publishing one hands a reader a kill
/// target no read surface has ever exposed.
#[test]
fn active_writers_lists_live_holders_only_and_never_discloses_a_pid() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());

    let writer = |worktree: &str| LeaseResource::writer("host", "/srv/harbard/.git", worktree);
    let expired = writer("/srv/harbard/worktrees/expired").unwrap();
    let orphaned = writer("/srv/harbard/worktrees/orphaned").unwrap();
    let held = writer("/srv/harbard").unwrap();
    let newest = writer("/srv/harbard/worktrees/newest").unwrap();

    // 60 s TTL from t=1_000 → expires at 61_000, well before the read below.
    store
        .claim(
            &expired,
            &metadata("gone", Some(11)),
            60,
            1_000,
            &FakePid(true),
        )
        .unwrap();
    store
        .claim(
            &held,
            &metadata("holder", Some(22)),
            900,
            2_000,
            &FakePid(true),
        )
        .unwrap();
    store
        .claim(
            &orphaned,
            &metadata("orphan", Some(33)),
            900,
            3_000,
            &FakePid(true),
        )
        .unwrap();
    store
        .claim(
            &newest,
            &metadata("newcomer", Some(44)),
            900,
            5_000,
            &FakePid(true),
        )
        .unwrap();
    store
        .claim(
            &LeaseResource::service("host", "browser").unwrap(),
            &metadata("daemon", Some(55)),
            900,
            4_000,
            &FakePid(true),
        )
        .unwrap();

    // t=100_000: the 60 s lease has lapsed and pid 33 no longer exists.
    let views = store.active_writers(100_000_u64, &DeadPid(33));

    assert_eq!(
        views
            .iter()
            .map(|view| view.worktree.as_str())
            .collect::<Vec<_>>(),
        vec!["/srv/harbard/worktrees/newest", "/srv/harbard"],
        "only live writer holders, most recently renewed first"
    );
    let held_view = &views[1];
    assert_eq!(held_view.session, "holder");
    assert_eq!(held_view.harness, "codex");
    assert_eq!(held_view.host, "host");
    assert_eq!(held_view.git_common_dir, "/srv/harbard/.git");
    assert_eq!(
        held_view.repo, "harbard",
        "repo label drops the .git suffix"
    );
    assert_eq!(held_view.branch.as_deref(), Some("codex/lease-foundation"));
    assert_eq!(held_view.claimed_at_ms, 2_000);
    assert_eq!(held_view.renewed_at_ms, 2_000);
    assert_eq!(held_view.expires_at_ms, 902_000);

    // The disclosed shape is pinned: a field added to LeaseRecord must never
    // reach a reader by default, and `pid` must never reach one at all.
    let mut keys: Vec<String> = serde_json::to_value(held_view)
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "branch",
            "claimed_at_ms",
            "expires_at_ms",
            "git_common_dir",
            "harness",
            "host",
            "renewed_at_ms",
            "repo",
            "session",
            "worktree",
        ]
    );
}

/// Listing is a read. `stats` opens by renaming legacy-salt files onto the
/// current hash; copying that into the listing path would make a plain HTTP
/// GET mutate authoritative lease state on disk.
#[test]
fn active_writers_does_not_rewrite_lease_files() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    let resource = resource();
    store
        .claim(
            &resource,
            &metadata("holder", Some(7)),
            900,
            1_000,
            &FakePid(true),
        )
        .unwrap();

    let directory = store.root().join("writer");
    let snapshot = |directory: &Path| -> Vec<(PathBuf, SystemTime)> {
        let mut rows: Vec<(PathBuf, SystemTime)> = fs::read_dir(directory)
            .unwrap()
            .flatten()
            .map(|entry| {
                let modified = entry.metadata().unwrap().modified().unwrap();
                (entry.path(), modified)
            })
            .collect();
        rows.sort();
        rows
    };
    let before = snapshot(&directory);

    assert_eq!(store.active_writers(2_000_u64, &FakePid(true)).len(), 1);

    assert_eq!(before, snapshot(&directory), "listing rewrites nothing");
}

/// A corrupt or unreadable lease file degrades the listing, never the request:
/// the healthy holders still come back, and `stats` stays the health surface.
#[test]
fn active_writers_skips_corrupt_records_instead_of_failing() {
    let home = TempHome::new();
    let store = LeaseStore::at(home.path());
    store
        .claim(
            &LeaseResource::writer("host", "/srv/harbard/.git", "/srv/harbard").unwrap(),
            &metadata("holder", Some(7)),
            900,
            1_000,
            &FakePid(true),
        )
        .unwrap();
    let directory = store.root().join("writer");
    fs::write(directory.join("not-a-hash.json"), br#"{"v":1}"#).unwrap();
    fs::write(directory.join("ignored.tmp"), b"partial").unwrap();

    let views = store.active_writers(2_000_u64, &FakePid(true));
    assert_eq!(views.len(), 1, "one healthy holder survives the debris");
    assert_eq!(views[0].worktree, "/srv/harbard");
    assert!(
        !store.stats(2_000_u64, &FakePid(true)).healthy,
        "the debris is still reported as unhealthy by the health surface"
    );
}

#[test]
fn the_key_hash_is_pinned_to_its_bytes_on_disk() {
    // This hash names the lease file. It is computed here against a value
    // produced by a plain SHA-256 of the same byte sequence, not by this
    // implementation quoting itself: a formatting change that silently shifts
    // it orphans every lease the operator already holds — which is why
    // `legacy_key_hash` exists at all.
    let resource = LeaseResource {
        kind: LeaseKind::Writer,
        host: "fixture-host".to_string(),
        service: None,
        git_common_dir: Some("/fixture/repo/.git".to_string()),
        worktree: Some("/fixture/worktree".to_string()),
    };
    assert_eq!(
        resource.key_hash(),
        "bdc0a37764ad0e5688ba10188b3998d665b8cf0761dccd98c4b963d97e9bb65e"
    );
}
