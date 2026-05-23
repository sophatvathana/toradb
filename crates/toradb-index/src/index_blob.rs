//! Shared header wrapper for on-disk index snapshots (BM25, vectors, HNSW, …).

use rkyv::api::high::{from_bytes, to_bytes, HighSerializer, HighValidator};
use rkyv::bytecheck::CheckBytes;
use rkyv::de::Pool;
use rkyv::rancor::{Error, Source, Strategy};
use rkyv::ser::allocator::ArenaHandle;
use rkyv::util::AlignedVec;
use rkyv::{Archive, Deserialize, Serialize};

/// On-disk payload encoding 
pub const INDEX_BLOB_FORMAT_VERSION: u8 = 1;

/// `magic` (4) + `version` (1) + padding (3) so the rkyv payload starts 8-byte aligned.
pub const INDEX_BLOB_HEADER_LEN: usize = 8;

pub fn encode<T>(magic: &[u8; 4], value: &T) -> Result<Vec<u8>, String>
where
    T: for<'a> Serialize<HighSerializer<AlignedVec, ArenaHandle<'a>, Error>>,
{
    let payload = to_bytes::<Error>(value).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(INDEX_BLOB_HEADER_LEN + payload.len());
    out.extend_from_slice(magic);
    out.push(INDEX_BLOB_FORMAT_VERSION);
    out.extend_from_slice(&[0u8; 3]);
    out.extend_from_slice(payload.as_ref());
    Ok(out)
}

pub fn decode<T>(magic: &[u8; 4], bytes: &[u8]) -> Result<T, String>
where
    T: Archive,
    T::Archived: for<'a> CheckBytes<HighValidator<'a, Error>>
        + Deserialize<T, Strategy<Pool, Error>>,
    Error: Source,
{
    if bytes.len() < INDEX_BLOB_HEADER_LEN {
        return Err("index blob too short".into());
    }
    if &bytes[..4] != magic {
        return Err("invalid index blob magic".into());
    }
    if bytes[4] != INDEX_BLOB_FORMAT_VERSION {
        return Err(format!(
            "unsupported index blob version {} (expected rkyv format {INDEX_BLOB_FORMAT_VERSION})",
            bytes[4]
        ));
    }
    let payload = &bytes[INDEX_BLOB_HEADER_LEN..];
    let mut aligned = AlignedVec::<16>::with_capacity(payload.len());
    aligned.extend_from_slice(payload);
    from_bytes::<T, Error>(aligned.as_ref()).map_err(|e| e.to_string())
}
