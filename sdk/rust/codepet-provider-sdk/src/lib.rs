//! Generated CodePet Host ↔ out-of-process Provider plugin SDK.

mod generated;

pub use generated::*;

/// Bounded transport limit for Provider methods that return complete conversation histories.
/// Both the Provider process and Host reader must opt into this limit explicitly.
pub const MAX_CONVERSATION_HISTORY_JSON_LINE_BYTES: usize = 16 * 1024 * 1024;
