//! Temp-dir fixtures for unit tests.
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);

/// A fresh, empty, canonical temp directory.
pub fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("disko-engine-{}-{}-{}", tag, std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::canonicalize(&d).unwrap()
}
