//! Content-hash duplicate detection, split out of main_window.

use std::path::{Path, PathBuf};

pub(super) fn compute_duplicate_set_from_dir(dir: &Path) -> std::collections::HashSet<PathBuf> {
    use std::collections::HashMap;

    let Ok(entries) = std::fs::read_dir(dir) else {
        return std::collections::HashSet::new();
    };

    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() || meta.len() == 0 {
            continue;
        }
        by_size.entry(meta.len()).or_default().push(entry.path());
    }

    let mut duplicates = std::collections::HashSet::new();
    for (_size, paths) in by_size {
        if paths.len() < 2 {
            continue;
        }
        let mut by_hash: HashMap<u64, Vec<PathBuf>> = HashMap::new();
        for path in &paths {
            if let Some(hash) = hash_file_contents(path) {
                by_hash.entry(hash).or_default().push(path.clone());
            }
        }
        for (_hash, group) in by_hash {
            if group.len() >= 2 {
                duplicates.extend(group);
            }
        }
    }
    duplicates
}

fn hash_file_contents(path: &Path) -> Option<u64> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).ok()?;
    let mut hash = 14695981039346656037u64;
    let mut buf = [0u8; 65536];

    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            return Some(hash);
        }
        hash = fnv1a_continue(hash, &buf[..n]);
    }
}

fn fnv1a_continue(mut hash: u64, data: &[u8]) -> u64 {
    for b in data {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    hash
}
