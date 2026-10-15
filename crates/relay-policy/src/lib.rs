//! Membership read gate and SurrealDB credential checks for the Scuffed Crew relay.
//!
//! strfry's write-policy plugin (`relay-policy` binary) decides which events are
//! stored. It does not see `REQ` subscriptions. strfry29 is also a write plugin
//! (NIP-29 group admin). Upstream strfry has no merged read-policy plugin, so
//! `relay-read-gate` sits in front of the published port and filters what a
//! connection is allowed to receive.

pub mod creds;
pub mod directory;
pub mod nip42;
pub mod proxy;
pub mod read_policy;
pub mod session;
