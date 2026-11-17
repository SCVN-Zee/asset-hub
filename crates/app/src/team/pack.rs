//! The pack codec of the thumbnail routes. It lives in the `asset_hub_pack` crate, which the Worker
//! shares, so both sides use one wire format and one set of test vectors.

pub use asset_hub_pack::{decode_pack, encode_pack, shard_of, SHARDS};
