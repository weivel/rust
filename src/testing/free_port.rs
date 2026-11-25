//! A port nothing listens on.

use std::net::{Ipv4Addr, TcpListener};

use super::TestingFailure;

/// A port of the loopback address nothing listens on at the moment it is answered: the system picks it for a listener
/// that is closed again before the answer.
pub fn free_port() -> Result<u16, TestingFailure> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .map_err(|error| TestingFailure::new(format!("no port could be bound: {error}")))?;
    let address = listener.local_addr().map_err(|error| {
        TestingFailure::new(format!("the bound port could not be read: {error}"))
    })?;
    Ok(address.port())
}
