//! The object store: immutable, content-addressed blobs, trees and changes under
//! `<store>/objects/<2 hex>/<rest>`, named by the BLAKE3 hash (hex) of their bytes.
//! Writes go through a `Batch`, which makes everything it wrote durable at once (see
//! `Batch::finish`). The change-graph helpers at the bottom are pure: they take a parents lookup.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs::{self, File};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// Full object id: 64 hex digits.
pub type Id = String;

/// Length of the ids concord prints.
pub const SHORT_ID: usize = 12;

/// The first `SHORT_ID` hex digits of an id.
pub fn short(id: &str) -> &str {
    id.get(..SHORT_ID).unwrap_or(id)
}

/// One path of a tree. A symlink is a blob of its target with `link: true`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub blob: Id,
    #[serde(default, skip_serializing_if = "is_false")]
    pub exec: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub link: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Relative path (`/`-separated) → entry, sorted.
pub type Tree = BTreeMap<String, Entry>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub tree: Id,
    pub parents: Vec<Id>,
    pub agent: String,
    pub message: String,
}

pub fn hash(bytes: &[u8]) -> Id {
    blake3::hash(bytes).to_hex().to_string()
}

pub struct Objects {
    dir: PathBuf,
}

impl Objects {
    /// The object store of a `.concord` dir.
    pub fn new(store: &Path) -> Self {
        Objects {
            dir: store.join("objects"),
        }
    }

    fn path(&self, id: &str) -> PathBuf {
        let (fan, rest) = id.split_at(id.len().min(2));
        self.dir.join(fan).join(rest)
    }

    pub fn has(&self, id: &str) -> bool {
        is_full_id(id) && self.path(id).is_file()
    }

    pub fn get(&self, id: &str) -> io::Result<Vec<u8>> {
        if !is_full_id(id) {
            return Err(invalid(format!("not an object id: {id}")));
        }
        fs::read(self.path(id)).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => {
                io::Error::new(e.kind(), format!("object {} is missing", short(id)))
            }
            _ => e,
        })
    }

    pub fn tree(&self, id: &str) -> io::Result<Tree> {
        serde_json::from_slice(&self.get(id)?).map_err(io::Error::other)
    }

    pub fn change(&self, id: &str) -> io::Result<Change> {
        serde_json::from_slice(&self.get(id)?).map_err(io::Error::other)
    }

    /// The tree a change points at.
    pub fn change_tree(&self, id: &str) -> io::Result<Tree> {
        self.tree(&self.change(id)?.tree)
    }

    /// The full id of the one object whose id starts with `prefix` (at least 2 hex digits).
    pub fn resolve(&self, prefix: &str) -> io::Result<Id> {
        let prefix = prefix.to_ascii_lowercase();
        if prefix.len() < 2 || prefix.len() > 64 || !prefix.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid(format!("not an object id: {prefix}")));
        }
        let (fan, rest) = prefix.split_at(2);
        let names = match fs::read_dir(self.dir.join(fan)) {
            Ok(names) => names,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(not_found(&prefix));
            }
            Err(e) => return Err(e),
        };
        let mut found = Vec::new();
        for name in names {
            let name = name?.file_name();
            if let Some(name) = name.to_str()
                && name.starts_with(rest)
            {
                found.push(format!("{fan}{name}"));
            }
        }
        match found.as_slice() {
            [one] if is_full_id(one) => Ok(one.clone()),
            [] => Err(not_found(&prefix)),
            _ => Err(invalid(format!("{prefix} names more than one object"))),
        }
    }

    pub fn batch(&self) -> Batch<'_> {
        Batch {
            objects: self,
            dirs: BTreeSet::new(),
            wrote: false,
        }
    }

    pub fn ancestors(&self, starts: &[Id]) -> io::Result<HashMap<Id, usize>> {
        ancestors(starts, |id| self.parents(id))
    }

    pub fn is_ancestor(&self, ancestor: &str, of: &str) -> io::Result<bool> {
        Ok(self.ancestors(&[of.to_string()])?.contains_key(ancestor))
    }

    pub fn lowest_common_ancestors(&self, a: &[Id], b: &[Id]) -> io::Result<Vec<Id>> {
        lowest_common_ancestors(a, b, |id| self.parents(id))
    }

    pub fn heads(&self, changes: &[Id]) -> io::Result<Vec<Id>> {
        heads(changes, |id| self.parents(id))
    }

    /// Whether everything `change` needs is here: the change, its ancestors, their trees and
    /// their blobs. The walk stops at `known`, changes the caller already knows are complete, so
    /// a new change costs its own new history, not the project's.
    pub fn complete(&self, change: &str, known: &BTreeSet<Id>) -> io::Result<bool> {
        let mut seen = BTreeSet::new();
        let mut todo = vec![change.to_string()];
        while let Some(id) = todo.pop() {
            if known.contains(&id) || !seen.insert(id.clone()) {
                continue;
            }
            if !self.has(&id) {
                return Ok(false);
            }
            let change = self.change(&id)?;
            if !self.has(&change.tree) {
                return Ok(false);
            }
            if self.tree(&change.tree)?.values().any(|e| !self.has(&e.blob)) {
                return Ok(false);
            }
            todo.extend(change.parents);
        }
        Ok(true)
    }

    fn parents(&self, id: &str) -> io::Result<Vec<Id>> {
        Ok(self.change(id)?.parents)
    }
}

/// Writes whose durability is settled together by `finish`.
pub struct Batch<'a> {
    objects: &'a Objects,
    /// Directories that got a new entry: each is synced once at the end.
    dirs: BTreeSet<PathBuf>,
    wrote: bool,
}

/// Distinguishes temp files of concurrent writers in one process.
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl Batch<'_> {
    /// Store `bytes`, return their id. An object already present is not written again.
    pub fn put(&mut self, bytes: &[u8]) -> io::Result<Id> {
        let id = hash(bytes);
        let path = self.objects.path(&id);
        if path.is_file() {
            return Ok(id);
        }
        let tmp_dir = self.objects.dir.join("tmp");
        fs::create_dir_all(&tmp_dir)?;
        let fan = path
            .parent()
            .ok_or_else(|| invalid("object path has no parent".into()))?;
        if !fan.is_dir() {
            fs::create_dir_all(fan)?;
            self.dirs.insert(self.objects.dir.clone());
        }
        let tmp = tmp_dir.join(format!(
            "{id}.{}.{}",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        rustix::fs::fsync(&file)?;
        drop(file);
        fs::rename(&tmp, &path)?;
        self.dirs.insert(fan.to_path_buf());
        self.wrote = true;
        Ok(id)
    }

    pub fn put_tree(&mut self, tree: &Tree) -> io::Result<Id> {
        self.put(&serde_json::to_vec(tree).map_err(io::Error::other)?)
    }

    pub fn put_change(&mut self, change: &Change) -> io::Result<Id> {
        self.put(&serde_json::to_vec(change).map_err(io::Error::other)?)
    }

    /// Make every object of this batch durable: each object file was synced when written; now
    /// each directory that got an entry is synced once, and on macOS one `F_FULLFSYNC` flushes
    /// the drive cache. Only after this may anything point at the objects.
    pub fn finish(self) -> io::Result<()> {
        if !self.wrote {
            return Ok(());
        }
        for dir in &self.dirs {
            rustix::fs::fsync(File::open(dir)?)?;
        }
        #[cfg(target_os = "macos")]
        rustix::fs::fcntl_fullfsync(File::open(&self.objects.dir)?)?;
        Ok(())
    }
}

fn is_full_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn not_found(prefix: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("no object {prefix}"))
}

// Change graph, pure over a parents lookup.

/// Every change reachable from `starts` (them included), with its BFS distance.
pub fn ancestors(
    starts: &[Id],
    mut parents: impl FnMut(&str) -> io::Result<Vec<Id>>,
) -> io::Result<HashMap<Id, usize>> {
    let mut seen = HashMap::new();
    let mut queue: VecDeque<(Id, usize)> = starts.iter().map(|id| (id.clone(), 0)).collect();
    while let Some((id, distance)) = queue.pop_front() {
        if seen.contains_key(&id) {
            continue;
        }
        for parent in parents(&id)? {
            queue.push_back((parent, distance + 1));
        }
        seen.insert(id, distance);
    }
    Ok(seen)
}

/// The lowest common ancestors of `a` and `b`: their common ancestors that are not an ancestor of
/// another common one, sorted. Several after criss-cross merges; none without shared history.
pub fn lowest_common_ancestors(
    a: &[Id],
    b: &[Id],
    mut parents: impl FnMut(&str) -> io::Result<Vec<Id>>,
) -> io::Result<Vec<Id>> {
    let from_a = ancestors(a, &mut parents)?;
    let from_b = ancestors(b, &mut parents)?;
    let common: BTreeSet<&Id> = from_a.keys().filter(|id| from_b.contains_key(*id)).collect();
    // Every ancestor of a common ancestor is common too, so one walk from all their parents
    // finds the common ones below another.
    let mut above = Vec::new();
    for id in &common {
        above.extend(parents(id)?);
    }
    let below = ancestors(&above, &mut parents)?;
    Ok(common
        .into_iter()
        .filter(|id| !below.contains_key(*id))
        .cloned()
        .collect())
}

/// The tips of `changes`: those that are not an ancestor of another one. Order kept, duplicates
/// dropped.
pub fn heads(
    changes: &[Id],
    mut parents: impl FnMut(&str) -> io::Result<Vec<Id>>,
) -> io::Result<Vec<Id>> {
    let mut below = BTreeSet::new();
    for id in changes {
        let proper: Vec<Id> = parents(id)?;
        below.extend(ancestors(&proper, &mut parents)?.into_keys());
    }
    let mut tips: Vec<Id> = Vec::new();
    for id in changes {
        if !below.contains(id) && !tips.contains(id) {
            tips.push(id.clone());
        }
    }
    Ok(tips)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::scratch;

    type T = io::Result<()>;

    #[test]
    fn objects_round_trip_and_resolve_by_short_id() -> T {
        let objects = Objects::new(&scratch("objects-roundtrip")?);
        let mut batch = objects.batch();
        let blob = batch.put(b"hello\n")?;
        let tree = Tree::from([(
            "a.txt".to_string(),
            Entry {
                blob: blob.clone(),
                exec: false,
                link: false,
            },
        )]);
        let tree_id = batch.put_tree(&tree)?;
        let change = Change {
            tree: tree_id,
            parents: vec![],
            agent: "a".into(),
            message: "first".into(),
        };
        let id = batch.put_change(&change)?;
        batch.finish()?;
        assert_eq!(objects.get(&blob)?, b"hello\n");
        assert_eq!(objects.change(&objects.resolve(short(&id))?)?, change);
        assert_eq!(objects.change_tree(&id)?, tree);
        assert_eq!(hash(&objects.get(&id)?), id);
        Ok(())
    }

    #[test]
    fn graph_helpers() -> T {
        // root <- a <- b, root <- c, m = merge(b, c)
        let graph: HashMap<&str, Vec<&str>> = HashMap::from([
            ("root", vec![]),
            ("a", vec!["root"]),
            ("b", vec!["a"]),
            ("c", vec!["root"]),
            ("m", vec!["b", "c"]),
        ]);
        let parents = |id: &str| -> io::Result<Vec<Id>> {
            Ok(graph
                .get(id)
                .map(|p| p.iter().map(|s| s.to_string()).collect())
                .unwrap_or_default())
        };
        let ids = |list: &[&str]| -> Vec<Id> { list.iter().map(|s| s.to_string()).collect() };
        assert_eq!(
            lowest_common_ancestors(&ids(&["b"]), &ids(&["c"]), parents)?,
            ids(&["root"])
        );
        assert_eq!(
            lowest_common_ancestors(&ids(&["m"]), &ids(&["b"]), parents)?,
            ids(&["b"])
        );
        assert_eq!(
            heads(&ids(&["a", "c", "b", "b"]), parents)?,
            ids(&["c", "b"])
        );
        assert!(ancestors(&ids(&["m"]), parents)?.contains_key("a"));
        assert!(!ancestors(&ids(&["c"]), parents)?.contains_key("a"));
        Ok(())
    }

    fn lcas(graph: &[(&str, &[&str])], a: &str, b: &str) -> io::Result<Vec<Id>> {
        let graph: HashMap<&str, &[&str]> = graph.iter().copied().collect();
        let parents = |id: &str| -> io::Result<Vec<Id>> {
            Ok(graph
                .get(id)
                .map(|p| p.iter().map(|s| s.to_string()).collect())
                .unwrap_or_default())
        };
        lowest_common_ancestors(&[a.to_string()], &[b.to_string()], parents)
    }

    #[test]
    fn criss_cross_merges_have_two_lowest_common_ancestors() -> T {
        // a2 = merge(a1, b1), b2 = merge(b1, a1).
        let graph: &[(&str, &[&str])] = &[
            ("root", &[]),
            ("a1", &["root"]),
            ("b1", &["root"]),
            ("a2", &["a1", "b1"]),
            ("b2", &["b1", "a1"]),
        ];
        assert_eq!(lcas(graph, "a2", "b2")?, ["a1", "b1"]);
        Ok(())
    }

    #[test]
    fn a_closer_older_ancestor_is_not_the_lowest() -> T {
        // a merged a side branch s off root; x (on a's long line) is the real lowest one.
        let graph: &[(&str, &[&str])] = &[
            ("root", &[]),
            ("x", &["root"]),
            ("s", &["root"]),
            ("p", &["x"]),
            ("q", &["p"]),
            ("r", &["q"]),
            ("a", &["r", "s"]),
            ("b", &["x"]),
        ];
        assert_eq!(lcas(graph, "a", "b")?, ["x"]);
        Ok(())
    }

    #[test]
    fn a_completeness_check_stops_at_changes_known_complete() -> T {
        let objects = Objects::new(&scratch("objects-complete-known")?);
        let mut batch = objects.batch();
        let tree = batch.put_tree(&Tree::new())?;
        let mut change = |parents: Vec<Id>, message: &str| {
            let (agent, message) = ("a".to_string(), message.to_string());
            batch.put_change(&Change { tree: tree.clone(), parents, agent, message })
        };
        // `old` is not in this store: walking past `known` would find it missing.
        let old = hash(b"an old change elsewhere");
        let known = change(vec![old], "known")?;
        let new = change(vec![known.clone()], "new")?;
        batch.finish()?;
        assert!(!objects.complete(&new, &BTreeSet::new())?);
        assert!(objects.complete(&new, &BTreeSet::from([known]))?);
        Ok(())
    }
}
