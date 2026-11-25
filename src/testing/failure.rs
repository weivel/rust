//! Why a helper of `testing` could not do what it was asked.

use std::fmt;

/// What went wrong, in English, naming the path or the declaration it concerns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestingFailure {
    /// What happened.
    pub detail: String,
}

impl TestingFailure {
    pub(crate) fn new(detail: impl Into<String>) -> TestingFailure {
        TestingFailure {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for TestingFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for TestingFailure {}
