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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn small_files_get_the_same_sampled_and_full_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("small.txt");
        fs::write(&path, "hello").unwrap();
        assert_eq!(sampled_hash(&path, 5).unwrap(), full_hash(&path).unwrap());
    }

    #[test]
    fn sampled_hash_sees_changes_in_the_sampled_blocks_only() {
        let dir = tempfile::tempdir().unwrap();
        let size = 4 * 1024 * 1024;
        let original = vec![7u8; size];
        let hash = |name: &str, data: &[u8]| {
            let path = dir.path().join(name);
            fs::write(&path, data).unwrap();
            sampled_hash(&path, data.len() as u64).unwrap()
        };
        let base = hash("base", &original);

        let mut end_changed = original.clone();
        end_changed[size - 1] = 0;
        assert_ne!(hash("end", &end_changed), base);

        let mut between_samples = original.clone();
        between_samples[size / 4] = 0;
        assert_eq!(hash("between", &between_samples), base, "a change between samples is not seen");
    }
}
