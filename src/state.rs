//! Durable redb state for the standalone artifact service.
#[path = "state/redb.rs"]
mod redb;

pub use redb::{
    Blob, CURRENT_SCHEMA, Garbage, Imported, Operation, PeerIdentity, Prepared, Reference, Root,
    State, StateTx,
};
