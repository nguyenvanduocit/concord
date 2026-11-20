//! Board persistence: one JSON file in the project's `.concord` dir, every access under an
//! exclusive `flock`; the ledger of landed rounds beside it.

use std::fs::{self, File};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::core::Board;
use crate::worktree::State;

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Store { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The project's main folder: the one holding this `.concord`.
    pub fn main_root(&self) -> Option<&Path> {
        self.dir.parent()
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join("state.json")
    }

    /// Run `f` inside the board lock. `f` gets the current board (None before `init`) and returns
    /// `(Some(new_board), value)` to persist a change, `(None, value)` to leave the board as is.
    pub fn transact<T>(
        &self,
        f: impl FnOnce(Option<Board>) -> (Option<Board>, T),
    ) -> io::Result<T> {
        fs::create_dir_all(&self.dir)?;
        let lock = File::create(self.dir.join("lock"))?;
        lock.lock()?;
        let current = read(&self.state_path())?;
        let (changed, value) = f(current);
        if let Some(board) = changed {
            let text = serde_json::to_vec_pretty(&board).map_err(io::Error::other)?;
            write_atomic(&self.state_path(), &text)?;
        }
        Ok(value)
        // `lock` drops here, releasing the flock after the write is in place.
    }

    /// Held while a gate runs: one gate at a time per project. Released when dropped.
    pub fn gate_lock(&self) -> io::Result<File> {
        fs::create_dir_all(&self.dir)?;
        let lock = File::create(self.dir.join("gate.lock"))?;
        lock.lock()?;
        Ok(lock)
    }

    pub fn load(&self) -> io::Result<Option<Board>> {
        self.transact(|board| (None, board))
    }

    /// Append one landed round to `ledger.jsonl`, synced before returning.
    pub fn append_ledger(&self, entry: &serde_json::Value) -> io::Result<()> {
        let mut line = serde_json::to_vec(entry).map_err(io::Error::other)?;
        line.push(b'\n');
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("ledger.jsonl"))?;
        file.write_all(&line)?;
        rustix::fs::fsync(&file)?;
        Ok(())
    }
}

/// Where a folder belongs: `folder` is the nearest one with `.concord/workspace.json` (the main
/// folder, an agent workspace or a joined clone), `board` the project's `.concord`, where the
/// board and the objects live (the agent workspace's `store`, else the folder's own).
#[derive(Debug, PartialEq, Eq)]
pub struct Place {
    pub folder: PathBuf,
    pub board: PathBuf,
}

pub fn locate(start: &Path) -> io::Result<Place> {
    let start = fs::canonicalize(start)?;
    // The nearest folder with `.concord` or `.git` decides: a git repo nested in a project (a test
    // fixture, a vendored repo) is a project of its own, never part of the enclosing board.
    let Some(folder) = start
        .ancestors()
        .find(|dir| dir.join(".concord/workspace.json").is_file() || dir.join(".git").exists())
        .filter(|dir| dir.join(".concord/workspace.json").is_file())
    else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "{} is not inside a concord project (run concord plan or concord init there first)",
                start.display()
            ),
        ));
    };
    let text = fs::read(folder.join(".concord/workspace.json"))?;
    let state: State = serde_json::from_slice(&text).map_err(io::Error::other)?;
    let board = state.store.unwrap_or_else(|| folder.join(".concord"));
    Ok(Place {
        folder: folder.to_path_buf(),
        board,
    })
}

fn read(path: &Path) -> io::Result<Option<Board>> {
    match fs::read(path) {
        Ok(text) => serde_json::from_slice(&text)
            .map(Some)
            .map_err(io::Error::other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Distinguishes temp files of concurrent writers in one process.
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Replace `path` with `bytes` so a crash leaves the old or the new content: write a temp file
/// of this writer's own (`<file>.<pid>.<n>.tmp`), sync it, rename it into place, sync the folder.
/// Concurrent writers each land whole; the last rename wins.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = path.with_file_name(name);
    let mut file = File::create(&tmp)?;
    file.write_all(bytes)?;
    rustix::fs::fsync(&file)?;
    drop(file);
    fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent() {
        rustix::fs::fsync(File::open(dir)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::scratch;
    use crate::worktree::Worktree;

    type T = io::Result<()>;

    #[test]
    fn a_workspace_and_its_subfolders_reach_the_project_board() -> T {
        let root = scratch("locate")?;
        let (main, ws) = (root.join("main"), root.join("main-a"));
        Worktree::create(&main, "main", None)?;
        Worktree::create(&ws, "a", Some(main.join(".concord")))?;
        fs::create_dir_all(ws.join("src/deep"))?;
        assert_eq!(
            locate(&main)?,
            Place {
                folder: main.clone(),
                board: main.join(".concord")
            }
        );
        assert_eq!(
            locate(&ws.join("src/deep"))?,
            Place {
                folder: ws,
                board: main.join(".concord")
            }
        );
        Ok(())
    }

    #[test]
    fn concurrent_atomic_writes_each_land_whole() -> T {
        let file = scratch("write-atomic")?.join("workspace.json");
        let writers: Vec<_> = (1..=8usize)
            .map(|n| {
                let file = file.clone();
                std::thread::spawn(move || -> io::Result<()> {
                    for _ in 0..50 {
                        write_atomic(&file, &vec![b'0' + n as u8; n * 4096])?;
                    }
                    Ok(())
                })
            })
            .collect();
        for writer in writers {
            writer.join().map_err(|_| io::Error::other("writer panicked"))??;
        }
        let bytes = fs::read(&file)?;
        let n = bytes.len() / 4096;
        assert_eq!(bytes, vec![b'0' + n as u8; n * 4096]);
        Ok(())
    }

    #[test]
    fn board_round_trips_under_the_lock() -> T {
        let store = Store::new(scratch("store-roundtrip")?.join("board"));
        assert_eq!(store.load()?, None);
        let board = crate::core::Board {
            team: vec!["a".into()],
            ..Default::default()
        };
        store.transact(|_| (Some(board.clone()), ()))?;
        assert_eq!(store.load()?, Some(board));
        Ok(())
    }
}
