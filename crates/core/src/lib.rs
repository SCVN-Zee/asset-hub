//! Pure domain logic of Asset Hub (the Rust side of `src/core`). It has no Tauri dependency and
//! does no I/O except through traits.
//!
//! Every module is declared here, empty, so that the porting work on the modules never has to edit
//! this file.

// `!(x > 0.0)` is deliberate where a NaN must take the same branch as in the TypeScript, and the byte
// readers are clearer as `chunks_exact` than as `as_chunks`.
#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::chunks_exact_to_as_chunks, clippy::type_complexity)]

pub mod audio;
pub mod catalog;
pub mod deps;
pub mod errors;
pub mod export;
pub mod font;
pub mod format;
pub mod github;
pub mod handlers;
pub mod image;
pub mod js;
pub mod registry;
pub mod search;
pub mod stats;
pub mod tree;
pub mod types;
pub mod unity;
