//! Source-specific parsing stays behind this business interface.
pub mod codex;
pub mod local;
use crate::{
    domain::{Cursor, SourceBatch, Thread},
    Result,
};

pub trait ConversationSource {
    fn discover(&self) -> Result<Vec<Thread>>;
    fn read(&self, thread: &Thread, cursor: &Cursor) -> Result<SourceBatch>;
}
