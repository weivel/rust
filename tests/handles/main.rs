//! The `handles` module's tests: room taken before an addition, entries by handle and by token, entries ended by
//! owner and kind, and holdings dropped by the revocation that names them.

use weivel_rust::core::Handle;
use weivel_rust::handles::{HandleKind, Handles};

type Table = Handles<&'static str, u32>;

#[test]
fn an_entry_is_found_by_its_handle_and_its_token_until_it_ends() {
    let table = Table::default();
    let entry = table
        .reserve()
        .unwrap()
        .add(HandleKind::Plugin, Handle(5), None, "agenda");
    assert_eq!(
        table.find(Handle(5)).map(|found| found.record),
        Some("agenda")
    );
    assert_eq!(
        table.find_by_token(entry.token).map(|found| found.handle),
        Some(Handle(5))
    );
    assert_eq!(
        table.end(Handle(5)).map(|ended| ended.record),
        Some("agenda")
    );
    assert!(table.find(Handle(5)).is_none());
    assert!(table.find_by_token(entry.token).is_none());
}

#[test]
fn a_token_is_never_reissued_for_a_newer_entry() {
    let table = Table::default();
    let first = table
        .reserve()
        .unwrap()
        .add(HandleKind::Plugin, Handle(1), None, "first");
    table.end(Handle(1));
    let second = table
        .reserve()
        .unwrap()
        .add(HandleKind::Plugin, Handle(1), None, "second");
    assert_ne!(first.token, second.token);
    assert!(table.find_by_token(first.token).is_none());
}

#[test]
fn ending_by_owner_ends_only_the_kinds_named() {
    let table = Table::default();
    let owner = Handle(1);
    for (handle, kind) in [
        (2, HandleKind::ServiceRegistration),
        (3, HandleKind::Watch),
        (4, HandleKind::Want),
    ] {
        table
            .reserve()
            .unwrap()
            .add(kind, Handle(handle), Some(owner), "owned");
    }
    let mut ended: Vec<u64> = table
        .end_owned(owner, &[HandleKind::ServiceRegistration, HandleKind::Watch])
        .into_iter()
        .map(|entry| entry.handle.0)
        .collect();
    ended.sort();
    assert_eq!(ended, vec![2, 3]);
    assert!(table.find(Handle(4)).is_some());
}

#[test]
fn a_holding_is_kept_by_its_pair_and_dropped_by_its_revocation() {
    let table = Table::default();
    table.reserve().unwrap().hold(Handle(1), Handle(9), 42);
    assert_eq!(table.held(Handle(1), Handle(9)), Some(42));
    assert_eq!(table.held(Handle(2), Handle(9)), None);
    assert_eq!(table.revoke(Handle(1), Handle(9)), Some(42));
    assert_eq!(table.holding_count(), 0);
}

#[test]
fn an_unused_reservation_gives_its_room_back() {
    let table = Table::default();
    drop(table.reserve().unwrap());
    table
        .reserve()
        .unwrap()
        .add(HandleKind::Watch, Handle(3), Some(Handle(1)), "watch");
    assert_eq!(table.entry_count(), 1);
}
