//! The suite's list of vectors held to the pinned kit both ways: every vector there has a test, and every test names a
//! vector there.

use std::collections::BTreeSet;

use crate::vectors::{SAT, SAT_WITH_ITS_OWN_TEST, vectors};

#[test]
fn every_vector_of_the_kit_has_a_test_and_no_test_names_another() {
    let in_the_kit: BTreeSet<String> = vectors().into_iter().map(|vector| vector.id).collect();
    let tested: BTreeSet<String> = SAT
        .iter()
        .chain(SAT_WITH_ITS_OWN_TEST)
        .map(|id| (*id).to_owned())
        .collect();
    assert_eq!(tested, in_the_kit);
    assert_eq!(tested.len(), 94);
}
