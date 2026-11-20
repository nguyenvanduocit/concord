//! Helpers shared by the unit tests.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Fresh scratch dir under target/ (the internal disk is nearly full; stay on the project drive).
pub fn scratch(name: &str) -> io::Result<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/test-tmp")
        .join(name);
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(&dir)?;
    fs::canonicalize(dir)
}
