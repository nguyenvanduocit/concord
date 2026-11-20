//! Three-way merge of trees. `plan` is the functional core: per path, what the merge does, decided
//! from the three trees alone. `apply` is the shell: it carries a plan out in a folder (mergiraf
//! for text merged on both sides) and lists the conflicts; `unresolved` tells which of them are
//! still unsettled.
//!
//! Settling: a `Text` conflict by removing the markers, a `Binary` one by deleting `<path>.theirs`.
//! A `ModifyDelete` keeps the modified file and is a decision, never a block: keeping the file and
//! deleting it both count as settled.

use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::objects::{Entry, Objects, Tree};
use crate::worktree::{remove_path, set_exec, write_entry};

/// What the merge does at one path. Paths where ours already is the result get no step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Only theirs changed a path ours has: write their entry, or delete the path (`None`).
    Take(Option<Entry>),
    /// Only theirs has the path (ours never had it): write their entry. Something already on disk
    /// there (an untracked or ignored file, a folder) stays, theirs goes to `<path>.theirs`.
    Add(Entry),
    /// Both changed the file: merge the three versions as text. `base` is `None` when both added
    /// it (an empty base). Bytes that turn out not to be UTF-8 get the `Binary` rule.
    Text3 {
        base: Option<Entry>,
        ours: Entry,
        theirs: Entry,
    },
    /// One side deleted the file, the other modified it: the modified file stays. `theirs` is
    /// their modified entry to write (the `Add` rule), `None` when ours is the modified one.
    ModifyDelete { theirs: Option<Entry> },
    /// Both changed it and it cannot be merged as text (a symlink on either side): ours stays,
    /// theirs is written next to it as `<path>.theirs`.
    Binary { theirs: Entry },
}

/// The merge of `theirs` into `ours`, both descended from `base`.
pub fn plan(base: &Tree, ours: &Tree, theirs: &Tree) -> Vec<(String, Step)> {
    let mut paths: Vec<&String> = base
        .keys()
        .chain(ours.keys())
        .chain(theirs.keys())
        .collect();
    paths.sort();
    paths.dedup();
    let mut steps = Vec::new();
    for path in paths {
        let (b, o, t) = (base.get(path), ours.get(path), theirs.get(path));
        if o == t || b == t {
            continue;
        }
        let step = if b == o {
            match (o, t) {
                (None, Some(t)) => Step::Add(t.clone()),
                _ => Step::Take(t.cloned()),
            }
        } else {
            match (o, t) {
                (Some(o), Some(t)) if o.link || t.link => Step::Binary { theirs: t.clone() },
                (Some(o), Some(t)) => Step::Text3 {
                    base: b.filter(|b| !b.link).cloned(),
                    ours: o.clone(),
                    theirs: t.clone(),
                },
                (Some(_), None) => Step::ModifyDelete { theirs: None },
                (None, Some(t)) => Step::ModifyDelete {
                    theirs: Some(t.clone()),
                },
                (None, None) => continue,
            }
        };
        steps.push((path.clone(), step));
    }
    steps
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    pub path: String,
    pub kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// Conflict markers in the file.
    Text,
    /// Modified on one side, deleted on the other: the modified file stays. A decision to make,
    /// settled whether the file is kept or deleted.
    ModifyDelete,
    /// Theirs is in `<path>.theirs` (a symlink or non-UTF-8 file changed on both sides, or theirs
    /// added a path something on disk already holds); settled by deleting that file.
    Binary,
}

/// The file theirs goes to when a `Binary` conflict keeps ours at `path`.
pub fn theirs_path(path: &str) -> String {
    format!("{path}.theirs")
}

/// Carry out `steps` in the folder `root`, whose files are ours. Returns the conflicts.
/// Deletions run first, so a folder theirs replaced by a file (or the reverse) is gone before
/// the write.
pub fn apply(
    objects: &Objects,
    root: &Path,
    steps: &[(String, Step)],
) -> io::Result<Vec<Conflict>> {
    for (path, step) in steps {
        if *step == Step::Take(None) {
            remove_path(root, path)?;
        }
    }
    let mut conflicts: Vec<Conflict> = Vec::new();
    let scratch = root.join(".concord/merge");
    for (path, step) in steps {
        let found = match step {
            Step::Take(None) => None,
            Step::Take(Some(entry)) => {
                write_entry(objects, root, path, entry)?;
                None
            }
            Step::Add(entry) | Step::ModifyDelete { theirs: Some(entry) } => {
                let kind = matches!(step, Step::ModifyDelete { .. }).then_some(Kind::ModifyDelete);
                add(objects, root, path, entry, kind)?
            }
            Step::ModifyDelete { theirs: None } => Some((path.clone(), Kind::ModifyDelete)),
            Step::Text3 { base, ours, theirs } => {
                let base_bytes = match base {
                    Some(base) => objects.get(&base.blob)?,
                    None => Vec::new(),
                };
                let ours_bytes = objects.get(&ours.blob)?;
                let theirs_bytes = objects.get(&theirs.blob)?;
                let text = [&base_bytes, &ours_bytes, &theirs_bytes]
                    .iter()
                    .all(|bytes| std::str::from_utf8(bytes).is_ok());
                if text {
                    fs::create_dir_all(&scratch)?;
                    let files = [
                        ("base", &base_bytes),
                        ("ours", &ours_bytes),
                        ("theirs", &theirs_bytes),
                    ];
                    for (name, bytes) in files {
                        fs::write(scratch.join(name), bytes)?;
                    }
                    let clean = mergiraf(&scratch, root, path)?;
                    let exec = match base {
                        Some(base) if base.exec == ours.exec => theirs.exec,
                        _ => ours.exec,
                    };
                    set_exec(&root.join(path), exec)?;
                    (!clean).then(|| (path.clone(), Kind::Text))
                } else {
                    write_entry(objects, root, &theirs_path(path), theirs)?;
                    Some((path.clone(), Kind::Binary))
                }
            }
            Step::Binary { theirs } => {
                write_entry(objects, root, &theirs_path(path), theirs)?;
                Some((path.clone(), Kind::Binary))
            }
        };
        if let Some((path, kind)) = found {
            let conflict = Conflict { path, kind };
            if !conflicts.contains(&conflict) {
                conflicts.push(conflict);
            }
        }
    }
    match fs::remove_dir_all(&scratch) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(conflicts),
    }
}

/// Write theirs at `path`, which ours does not have. When something on disk holds the path or a
/// file holds one of its folders (`clash`), that stays and theirs goes under `<clash>.theirs`:
/// a `Binary` conflict at `clash`. Otherwise the conflict is `kind`, if any.
fn add(
    objects: &Objects,
    root: &Path,
    path: &str,
    entry: &Entry,
    kind: Option<Kind>,
) -> io::Result<Option<(String, Kind)>> {
    let Some(clash) = clash(root, path)? else {
        write_entry(objects, root, path, entry)?;
        return Ok(kind.map(|kind| (path.to_string(), kind)));
    };
    let rest = path.get(clash.len()..).unwrap_or_default();
    let target = format!("{}{rest}", theirs_path(&clash));
    write_entry(objects, root, &target, entry)?;
    Ok(Some((clash, Kind::Binary)))
}

/// The prefix of `path` in the way of writing it: `path` itself when anything is there, else the
/// nearest folder of it that is a file or symlink.
fn clash(root: &Path, path: &str) -> io::Result<Option<String>> {
    let mut prefix = String::new();
    let parts: Vec<&str> = path.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            prefix.push('/');
        }
        prefix.push_str(part);
        let last = i + 1 == parts.len();
        match root.join(&prefix).symlink_metadata() {
            Ok(meta) if last || !meta.is_dir() => return Ok(Some(prefix)),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// `mergiraf merge` of the three versions in `scratch` into `root/path`. True when clean.
fn mergiraf(scratch: &Path, root: &Path, path: &str) -> io::Result<bool> {
    let output = Command::new("mergiraf")
        .arg("merge")
        .args([
            scratch.join("base"),
            scratch.join("ours"),
            scratch.join("theirs"),
        ])
        .args(["-s", "base", "-x", "ours", "-y", "theirs", "-p", path, "-o"])
        .arg(root.join(path))
        .output()
        .map_err(|e| io::Error::new(e.kind(), format!("cannot run mergiraf: {e}")))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(io::Error::other(format!(
            "mergiraf failed on {path}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

/// The conflicts in `listed` that are still unsettled in the folder `root`.
pub fn unresolved(root: &Path, listed: &[Conflict]) -> io::Result<Vec<Conflict>> {
    let mut open = Vec::new();
    for conflict in listed {
        let path = root.join(&conflict.path);
        let still = match conflict.kind {
            Kind::Text => match fs::read(&path) {
                Ok(bytes) => has_markers(&bytes),
                Err(e) if e.kind() == io::ErrorKind::NotFound => false,
                Err(e) => return Err(e),
            },
            Kind::ModifyDelete => false,
            Kind::Binary => root
                .join(theirs_path(&conflict.path))
                .symlink_metadata()
                .is_ok(),
        };
        if still {
            open.push(conflict.clone());
        }
    }
    Ok(open)
}

/// A line starting with a conflict marker `<<<<<<< `.
pub fn has_markers(bytes: &[u8]) -> bool {
    bytes
        .split(|&b| b == b'\n')
        .any(|line| line.starts_with(b"<<<<<<< "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::scratch;
    use crate::worktree::Worktree;

    fn entry(blob: &str) -> Entry {
        Entry {
            blob: blob.into(),
            exec: false,
            link: false,
        }
    }

    fn tree(entries: &[(&str, Entry)]) -> Tree {
        entries
            .iter()
            .map(|(path, e)| (path.to_string(), e.clone()))
            .collect()
    }

    #[test]
    fn plan_cases() {
        let link = Entry {
            link: true,
            ..entry("l2")
        };
        let base = tree(&[
            ("same", entry("s")),
            ("ours-only", entry("a")),
            ("theirs-only", entry("b")),
            ("both-same", entry("c")),
            ("both-diff", entry("d")),
            ("we-delete", entry("e")),
            ("they-delete", entry("f")),
            ("del-mod", entry("g")),
            ("mod-del", entry("h")),
            ("link", entry("l")),
        ]);
        let ours = tree(&[
            ("same", entry("s")),
            ("ours-only", entry("a2")),
            ("theirs-only", entry("b")),
            ("both-same", entry("c2")),
            ("both-diff", entry("d2")),
            ("they-delete", entry("f")),
            ("mod-del", entry("h2")),
            ("added", entry("x")),
            ("link", entry("l1")),
        ]);
        let theirs = tree(&[
            ("same", entry("s")),
            ("ours-only", entry("a")),
            ("theirs-only", entry("b2")),
            ("both-same", entry("c2")),
            ("both-diff", entry("d3")),
            ("we-delete", entry("e")),
            ("del-mod", entry("g2")),
            ("added", entry("y")),
            ("link", link.clone()),
            ("theirs-add", entry("z")),
        ]);
        let steps = plan(&base, &ours, &theirs);
        let expected = vec![
            (
                "added".to_string(),
                Step::Text3 {
                    base: None,
                    ours: entry("x"),
                    theirs: entry("y"),
                },
            ),
            (
                "both-diff".into(),
                Step::Text3 {
                    base: Some(entry("d")),
                    ours: entry("d2"),
                    theirs: entry("d3"),
                },
            ),
            (
                "del-mod".into(),
                Step::ModifyDelete {
                    theirs: Some(entry("g2")),
                },
            ),
            ("link".into(), Step::Binary { theirs: link }),
            ("mod-del".into(), Step::ModifyDelete { theirs: None }),
            ("theirs-add".into(), Step::Add(entry("z"))),
            ("theirs-only".into(), Step::Take(Some(entry("b2")))),
            ("they-delete".into(), Step::Take(None)),
        ];
        assert_eq!(steps, expected);
    }

    fn stored(objects: &Objects, bytes: &[u8]) -> io::Result<Entry> {
        let mut batch = objects.batch();
        let blob = batch.put(bytes)?;
        batch.finish()?;
        Ok(entry(&blob))
    }

    #[test]
    fn an_added_file_never_overwrites_an_untracked_one() -> io::Result<()> {
        let root = scratch("merge-untracked")?;
        let objects = Objects::new(&root.join(".concord"));
        fs::write(root.join(".env"), "secret\n")?;
        let theirs = tree(&[(".env", stored(&objects, b"theirs\n")?)]);
        let conflicts = apply(&objects, &root, &plan(&Tree::new(), &Tree::new(), &theirs))?;
        assert_eq!(fs::read_to_string(root.join(".env"))?, "secret\n");
        assert_eq!(fs::read_to_string(root.join(".env.theirs"))?, "theirs\n");
        let binary = vec![Conflict { path: ".env".into(), kind: Kind::Binary }];
        assert_eq!(conflicts, binary);
        Ok(())
    }

    #[test]
    fn a_folder_replaced_by_a_file_merges_and_a_clash_is_a_conflict() -> io::Result<()> {
        let root = scratch("merge-folder-file")?;
        let objects = Objects::new(&root.join(".concord"));
        let x = stored(&objects, b"x\n")?;
        let foo = stored(&objects, b"foo\n")?;
        let ours = tree(&[("foo/x", x)]);
        let theirs = tree(&[("foo", foo)]);
        fs::create_dir_all(root.join("foo"))?;
        fs::write(root.join("foo/x"), "x\n")?;

        // Only theirs replaced the folder: it goes, the file comes.
        assert_eq!(apply(&objects, &root, &plan(&ours, &ours, &theirs))?, vec![]);
        assert_eq!(fs::read_to_string(root.join("foo"))?, "foo\n");

        // Ours added the folder, theirs the file: a conflict, ours stays.
        fs::remove_file(root.join("foo"))?;
        fs::create_dir_all(root.join("foo"))?;
        fs::write(root.join("foo/x"), "x\n")?;
        let conflicts = apply(&objects, &root, &plan(&Tree::new(), &ours, &theirs))?;
        assert_eq!(conflicts, vec![Conflict { path: "foo".into(), kind: Kind::Binary }]);
        assert_eq!(fs::read_to_string(root.join("foo/x"))?, "x\n");
        assert_eq!(fs::read_to_string(root.join("foo.theirs"))?, "foo\n");
        Ok(())
    }

    #[test]
    fn a_modify_delete_is_settled_whether_the_file_is_kept_or_deleted() -> io::Result<()> {
        let root = scratch("merge-modify-delete")?;
        let objects = Objects::new(&root.join(".concord"));
        let base = tree(&[("f", stored(&objects, b"1\n")?)]);
        let ours = tree(&[("f", stored(&objects, b"2\n")?)]);
        fs::write(root.join("f"), "2\n")?;
        let conflicts = apply(&objects, &root, &plan(&base, &ours, &Tree::new()))?;
        assert_eq!(conflicts, vec![Conflict { path: "f".into(), kind: Kind::ModifyDelete }]);
        assert_eq!(fs::read_to_string(root.join("f"))?, "2\n");
        assert_eq!(unresolved(&root, &conflicts)?, vec![]);
        fs::remove_file(root.join("f"))?;
        assert_eq!(unresolved(&root, &conflicts)?, vec![]);
        Ok(())
    }

    #[test]
    fn markers_are_lines_starting_with_the_marker() {
        assert!(has_markers(b"a\n<<<<<<< ours\nb\n"));
        assert!(!has_markers(b"a <<<<<<< b\n"));
    }

    #[test]
    fn a_real_conflict_is_listed_with_markers() -> io::Result<()> {
        let root = scratch("merge-conflict")?;
        let mut main = Worktree::create(&root.join("main"), "main", None)?;
        fs::write(root.join("main/f.txt"), "one\n")?;
        let base = main.commit("main", "base")?;
        let store = root.join("main/.concord");
        let mut other = Worktree::create(&root.join("other"), "other", Some(store))?;
        other.materialize(&base)?;
        fs::write(root.join("other/f.txt"), "theirs\n")?;
        let theirs = other.commit("other", "theirs")?;
        fs::write(root.join("main/f.txt"), "ours\n")?;

        let conflicts = main.merge_in(&[theirs])?;
        let expected = vec![Conflict {
            path: "f.txt".into(),
            kind: Kind::Text,
        }];
        assert_eq!(conflicts, expected);
        assert!(has_markers(&fs::read(root.join("main/f.txt"))?));
        assert_eq!(unresolved(&root.join("main"), &conflicts)?, expected);
        fs::write(root.join("main/f.txt"), "settled\n")?;
        assert_eq!(unresolved(&root.join("main"), &conflicts)?, vec![]);
        Ok(())
    }
}
