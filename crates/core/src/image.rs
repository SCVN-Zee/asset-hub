//! Decoders for the formats browsers cannot show (`src/core/image`).

pub mod psd;
pub mod tiff;

use serde::Serialize;

use crate::js::RangeError;

/// A decoded picture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// RGBA, 8 bits per channel, row-major.
    pub data: Vec<u8>,
}

/// Where the TypeScript decoders throw.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImageError {
    /// A read past the end of a truncated file (a `RangeError` in JavaScript).
    #[error(transparent)]
    Range(#[from] RangeError),
    /// Deflate data that does not inflate.
    #[error("cannot inflate: {0}")]
    Inflate(String),
    /// A picture that would need more memory than a decoder may ask for.
    #[error("image too large")]
    TooLarge,
}

/// The most memory one decoder buffer may take.
pub(crate) const MAX_BUFFER: u64 = 1 << 30;

pub(crate) fn buffer(len: u64) -> Result<Vec<u8>, ImageError> {
    if len > MAX_BUFFER {
        return Err(ImageError::TooLarge);
    }
    Ok(vec![0; len as usize])
}
