//! zstd, the one compression the container uses.
//!
//! A frame's stored payload is either the raw bytes or exactly one zstd
//! frame, and nothing here knows what those bytes mean. Natively this is the
//! C library; in a browser it is `ruzstd`, a pure-Rust decoder, because a
//! wasm build cannot link the C one. A browser never writes a `.worldview`,
//! so wasm has no encoder and asking for one is an error rather than a silent
//! pass-through.

#[cfg(target_arch = "wasm32")]
use std::io::Cursor;
use std::io::Read;

#[cfg(target_arch = "wasm32")]
use ruzstd::decoding::StreamingDecoder;
#[cfg(not(target_arch = "wasm32"))]
use zstd::stream::read::Decoder;
#[cfg(not(target_arch = "wasm32"))]
use zstd::zstd_safe::{find_frame_compressed_size, get_error_name};

use crate::container::error::{Error, Result};

/// The most a frame's payload may decompress to. The fattest payload a
/// producer writes is a scenario block carrying a world document, a few
/// megabytes; sixty-four mebibytes is far above it. Past the ceiling the
/// bytes are not something this container wrote, and a reader that kept going
/// would allocate until the process died — on a browser's 32-bit heap, long
/// before the machine noticed.
pub const MAX_DECOMPRESSED_LEN: usize = 64 * 1024 * 1024;

/// The level every payload is compressed at: compressing one frame at a time
/// at level 3 lands within a couple of percent of compressing a whole
/// keyframe cadence together, so the container compresses each frame alone
/// and does nothing cleverer.
pub const LEVEL: i32 = 3;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn compress(raw: &[u8]) -> Result<Vec<u8>> {
    zstd::stream::encode_all(raw, LEVEL).map_err(|e| Error::Compress { detail: e.to_string() })
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn compress(_raw: &[u8]) -> Result<Vec<u8>> {
    Err(Error::CompressionUnavailable)
}

/// Decode the one zstd frame a stored payload is, and refuse a payload that
/// holds anything else.
///
/// A payload the writer made is exactly one frame and ends where the frame
/// ends. Bytes after that frame — a second frame appended, or junk — would
/// otherwise decode to the first frame's contents and be handed on as if the
/// rest were not there, which is a silent change of what the frame says. The
/// native decoder is told where the frame ends before it starts; the wasm one
/// decodes from a cursor and the cursor says where it stopped.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn decompress(compressed: &[u8], offset: u64) -> Result<Vec<u8>> {
    let frame_len = find_frame_compressed_size(compressed)
        .map_err(|code| Error::Decompress { offset, detail: get_error_name(code).to_owned() })?;
    if frame_len != compressed.len() {
        return Err(one_frame_only(offset, frame_len, compressed.len()));
    }
    let decoder =
        Decoder::new(compressed).map_err(|e| Error::Decompress { offset, detail: e.to_string() })?.single_frame();
    under_the_ceiling(decoder, offset)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn decompress(compressed: &[u8], offset: u64) -> Result<Vec<u8>> {
    let mut source = Cursor::new(compressed);
    let decoder =
        StreamingDecoder::new(&mut source).map_err(|e| Error::Decompress { offset, detail: e.to_string() })?;
    let raw = under_the_ceiling(decoder, offset)?;
    let read = source.position() as usize;
    match read == compressed.len() {
        true => Ok(raw),
        false => Err(one_frame_only(offset, read, compressed.len())),
    }
}

fn one_frame_only(offset: u64, frame_len: usize, stored: usize) -> Error {
    Error::Decompress {
        offset,
        detail: format!("a stored payload is one zstd frame; this frame ends at byte {frame_len} of {stored}"),
    }
}

/// Read a decoder out, stopping one byte past [`MAX_DECOMPRESSED_LEN`]. The
/// bytes arrive as they decode, so what a frame claims about its size never
/// reserves anything, and a stream that keeps producing is dropped at the
/// ceiling instead of taking the heap with it.
fn under_the_ceiling<R: Read>(decoder: R, offset: u64) -> Result<Vec<u8>> {
    let mut raw = Vec::new();
    decoder
        .take(MAX_DECOMPRESSED_LEN as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|e| Error::Decompress { offset, detail: e.to_string() })?;
    match raw.len() > MAX_DECOMPRESSED_LEN {
        true => Err(Error::DecompressedTooLarge { offset, limit: MAX_DECOMPRESSED_LEN }),
        false => Ok(raw),
    }
}

#[cfg(test)]
mod tests {
    use std::io::repeat;

    use super::*;

    #[test]
    fn a_compressed_payload_comes_back_byte_for_byte() {
        let raw: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        let packed = compress(&raw).expect("compresses");
        assert!(packed.len() < raw.len(), "repetitive bytes should shrink");
        assert_eq!(decompress(&packed, 0).expect("decompresses"), raw);
    }

    #[test]
    fn a_payload_decompressing_past_the_ceiling_is_refused_and_names_its_frame() {
        // Compressed from a reader, so the test never holds the bomb it
        // builds; a run of zeros is what makes a few bytes into 64 MiB.
        let zeros = repeat(0u8).take(MAX_DECOMPRESSED_LEN as u64 + 1);
        let packed = zstd::stream::encode_all(zeros, LEVEL).expect("compresses");
        assert!(packed.len() < 8_192, "the bomb is {} bytes on the wire", packed.len());
        let error = decompress(&packed, 512).expect_err("past the ceiling");
        assert!(matches!(error, Error::DecompressedTooLarge { offset: 512, .. }), "got {error}");
    }

    #[test]
    fn a_payload_holding_more_than_one_zstd_frame_is_damage() {
        let mut packed = compress(b"the first frame").expect("compresses");
        packed.extend(compress(b"and a second one").expect("compresses"));
        let error = decompress(&packed, 96).expect_err("a stored payload is one frame");
        assert!(matches!(error, Error::Decompress { offset: 96, .. }), "got {error}");
    }

    #[test]
    fn bytes_after_the_frame_are_damage_rather_than_padding() {
        let mut packed = compress(b"the only frame").expect("compresses");
        packed.push(0);
        let error = decompress(&packed, 0).expect_err("nothing accounts for the byte after the frame");
        assert!(matches!(error, Error::Decompress { offset: 0, .. }), "got {error}");
    }

    #[test]
    fn decompressing_rubbish_names_the_offset() {
        let error = decompress(b"not a zstd frame", 4096).expect_err("rubbish is not a zstd frame");
        assert!(matches!(error, Error::Decompress { offset: 4096, .. }), "got {error}");
    }
}
