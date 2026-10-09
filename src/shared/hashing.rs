//! Content hashing with BLAKE3: full hash, or a sampled hash that reads only a few blocks.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

pub type Digest = [u8; 32];

/// Size of each sampled block.
const SAMPLE_SIZE: u64 = 16 * 1024;

/// Files up to this size are always hashed completely by the sampled hash.
const FULL_HASH_LIMIT: u64 = 1024 * 1024;

/// Hash of the whole file.
pub fn full_hash(path: &Path) -> io::Result<Digest> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(*hasher.finalize().as_bytes())
}

/// Hash of the size plus three blocks: start, middle and end. Small files are hashed fully.
pub fn sampled_hash(path: &Path, size: u64) -> io::Result<Digest> {
    if size <= FULL_HASH_LIMIT {
        return full_hash(path);
    }
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(&size.to_le_bytes());
    let mut buffer = vec![0u8; SAMPLE_SIZE as usize];
    for offset in [0, size / 2 - SAMPLE_SIZE / 2, size - SAMPLE_SIZE] {
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut buffer)?;
        hasher.update(&buffer);
    }
    Ok(*hasher.finalize().as_bytes())
}
