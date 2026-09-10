//! Evidence-backed task projections. Source files are always read-only.
pub mod domain;
pub mod extraction;
pub mod git;
pub mod management;
pub mod service;
pub mod sources;
pub mod store;
pub mod watch;

pub type Result<T> = std::result::Result<T, String>;

pub fn stable_id(value: impl AsRef<[u8]>) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(value.as_ref()))
}
