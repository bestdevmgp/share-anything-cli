pub mod error;
pub mod p2p;
pub mod upload;
pub mod download;
pub mod shares;
pub mod auth;

use std::sync::Arc;
pub type ProgressFn = Arc<dyn Fn(u64) + Send + Sync>;
