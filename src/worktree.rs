//! A folder concord works in: its `.concord/` state, snapshots of the folder into changes,
//! materializing a change into the folder, and merging other changes in.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::merge::{self, Conflict};
use crate::objects::{Batch, Change, Entry, Id, Objects, Tree};
use crate::store::write_atomic;

/// `.concord/workspace.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub name: String,
    /// The folder's last change; none before the project's first base.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<Id>,
    /// Changes merged in since `head`: the next change's other parents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merged: Vec<Id>,
    /// The project's `.concord` for agent workspaces; absent where the store is this folder's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<PathBuf>,
    /// What the last merges left to settle (`unresolved` tells which still are).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<Conflict>,
}

/// One stat-cache line of `.concord/index.json`: a file with this size, mtime, ctime and inode
/// has this blob. A rewrite that restores size and mtime still changes the ctime.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Cached {
    size: u64,
    mtime: u64,
    ctime: u64,
    ino: u64,
    blob: Id,
}

type Index = BTreeMap<String, Cached>;

/// A file modified this recently may change again within the same mtime tick: not cached.
const RACY: Duration = Duration::from_secs(2);

/// Names never tracked, at any depth.
const SKIPPED: [&str; 3] = [".concord", ".git", ".jj"];

pub struct Worktree {
    root: PathBuf,
    state: State,
    objects: Objects,
}

impl Worktree {
    /// Start `.concord/` in `root` (created if needed). `store` is the project's `.concord` for an
    /// agent workspace, `None` for a folder with its own store.
    pub fn create(root: &Path, name: &str, store: Option<PathBuf>) -> io::Result<Self> {
        let dot = root.join(".concord");
        fs::create_dir_all(&dot)?;
        fs::write(dot.join(".gitignore"), "*\n")?;
        let state = State {
            name: name.to_string(),
            head: None,
            merged: Vec::new(),
            store,
            conflicts: Vec::new(),
        };
        let worktree = Self::with_state(root, state);
        worktree.save()?;
        Ok(worktree)
    }

    pub fn open(root: &Path) -> io::Result<Self> {
        let text = fs::read(root.join(".concord/workspace.json"))?;
        let state = serde_json::from_slice(&text).map_err(io::Error::other)?;
        Ok(Self::with_state(root, state))
    }

    fn with_state(root: &Path, state: State) -> Self {
        let store = state.store.clone().unwrap_or_else(|| root.join(".concord"));
        Worktree {
            root: root.to_path_buf(),
            objects: Objects::new(&store),
            state,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn objects(&self) -> &Objects {
        &self.objects
    }

    fn dot(&self) -> PathBuf {
        self.root.join(".concord")
    }

    fn save(&self) -> io::Result<()> {
        let text = serde_json::to_vec_pretty(&self.state).map_err(io::Error::other)?;
        write_atomic(&self.dot().join("workspace.json"), &text)
    }

    /// The folder as a tree; its blobs are stored and durable.
    pub fn snapshot(&self) -> io::Result<Tree> {
        let mut batch = self.objects.batch();
        let (tree, index) = self.scan(&mut batch)?;
        batch.finish()?;
        self.save_index(index)?;
        Ok(tree)
    }

    /// Snapshot the folder into a change with parents `[head] + merged` (only their tips).
    pub fn commit(&mut self, agent: &str, message: &str) -> io::Result<Id> {
        let mut batch = self.objects.batch();
        let (tree, index) = self.scan(&mut batch)?;
        let tree = batch.put_tree(&tree)?;
        let parents = self.objects.heads(&self.parents())?;
        let change = batch.put_change(&Change {
            tree,
            parents,
            agent: agent.to_string(),
            message: message.to_string(),
        })?;
        batch.finish()?;
        self.save_index(index)?;
        self.state.head = Some(change.clone());
        self.state.merged.clear();
        self.state.conflicts.clear();
        self.save()?;
        Ok(change)
    }

    /// Make the folder hold `change`'s tree: write the paths whose entry differs, delete tracked
    /// paths the tree lacks. Ignored files are not touched. `change` becomes the head.
    pub fn materialize(&mut self, change: &str) -> io::Result<()> {
        let target = self.objects.change_tree(change)?;
        let mut batch = self.objects.batch();
        let (current, mut index) = self.scan(&mut batch)?;
        batch.finish()?;
        for path in current.keys().filter(|path| !target.contains_key(*path)) {
            remove_path(&self.root, path)?;
            index.remove(path);
        }
        for (path, entry) in &target {
            if current.get(path) != Some(entry) {
                write_entry(&self.objects, &self.root, path, entry)?;
                index.remove(path);
            }
        }
        self.save_index(index)?;
        self.state.head = Some(change.to_string());
        self.state.merged.clear();
        self.state.conflicts.clear();
        self.save()
    }

    /// Merge each of `changes` into the folder (3-way, base: `merge_base`) and record it in
    /// `merged`. A change already in the folder's history is skipped. Returns the
    /// conflicts.
    pub fn merge_in(&mut self, changes: &[Id]) -> io::Result<Vec<Conflict>> {
        let mut conflicts = Vec::new();
        for theirs in changes {
            let ours = self.parents();
            if self.objects.ancestors(&ours)?.contains_key(theirs) {
                continue;
            }
            let base = self.merge_base(&ours, std::slice::from_ref(theirs))?;
            let steps = merge::plan(&base, &self.snapshot()?, &self.objects.change_tree(theirs)?);
            let found = merge::apply(&self.objects, &self.root, &steps)?;
            self.state.conflicts.extend(found.iter().cloned());
            conflicts.extend(found);
            self.state.merged.push(theirs.clone());
            self.save()?;
        }
        Ok(conflicts)
    }

    /// The tree to merge `ours` and `theirs` against: their lowest common ancestor's tree. After
    /// criss-cross merges there are several; then, as git's recursive strategy does, they are
    /// merged pairwise (each pair against its own merge base) into a virtual base.
    fn merge_base(&self, ours: &[Id], theirs: &[Id]) -> io::Result<Tree> {
        let lcas = self.objects.lowest_common_ancestors(ours, theirs)?;
        let Some((first, rest)) = lcas.split_first() else {
            return Ok(Tree::new());
        };
        let mut done = vec![first.clone()];
        let mut tree = self.objects.change_tree(first)?;
        for next in rest {
            let base = self.merge_base(&done, std::slice::from_ref(next))?;
            tree = self.virtual_merge(&base, &tree, &self.objects.change_tree(next)?)?;
            done.push(next.clone());
        }
        Ok(tree)
    }

    /// `merge::plan` + `merge::apply` of three trees in a scratch folder, read back as a tree. Text
    /// conflicts keep their markers; every other conflict keeps the modified or ours side.
    fn virtual_merge(&self, base: &Tree, ours: &Tree, theirs: &Tree) -> io::Result<Tree> {
        let dir = self.dot().join("virtual-base");
        remove_dir(&dir)?;
        fs::create_dir_all(&dir)?;
        let steps = merge::plan(base, ours, theirs);
        for (path, _) in &steps {
            if let Some(entry) = ours.get(path) {
                write_entry(&self.objects, &dir, path, entry)?;
            }
        }
        merge::apply(&self.objects, &dir, &steps)?;
        let mut tree = ours.clone();
        let mut batch = self.objects.batch();
        for (path, _) in &steps {
            tree.remove(path);
            let file = dir.join(path);
            if let Ok(meta) = file.symlink_metadata()
                && let Some((entry, _)) = read_entry(&file, &meta, None, &mut batch)?
            {
                tree.insert(path.clone(), entry);
            }
        }
        batch.finish()?;
        remove_dir(&dir)?;
        Ok(tree)
    }

    /// Pull `theirs` into the folder as uncommitted edits: what changed from `base` to `theirs` is
    /// merged into the files as they are now, nothing else is touched. `theirs` becomes the head.
    /// Returns the conflicts.
    pub fn pull(&mut self, base: &str, theirs: &str) -> io::Result<Vec<Conflict>> {
        let steps = merge::plan(
            &self.objects.change_tree(base)?,
            &self.snapshot()?,
            &self.objects.change_tree(theirs)?,
        );
        let conflicts = merge::apply(&self.objects, &self.root, &steps)?;
        self.state.head = Some(theirs.to_string());
        self.state.merged.clear();
        self.state.conflicts = conflicts.clone();
        self.save()?;
        Ok(conflicts)
    }

    /// The listed conflicts not settled yet in the folder.
    pub fn unresolved(&self) -> io::Result<Vec<Conflict>> {
        merge::unresolved(&self.root, &self.state.conflicts)
    }

    /// Whether a commit now would record anything: merges, or files that differ from the head.
    pub fn has_work(&self) -> io::Result<bool> {
        if !self.state.merged.is_empty() {
            return Ok(true);
        }
        let head = match &self.state.head {
            Some(head) => self.objects.change_tree(head)?,
            None => Tree::new(),
        };
        Ok(self.snapshot()? != head)
    }

    fn parents(&self) -> Vec<Id> {
        self.state
            .head
            .iter()
            .chain(&self.state.merged)
            .cloned()
            .collect()
    }

    /// Walk the folder: every regular file and symlink the ignore files do not exclude, outside
    /// `SKIPPED`, plus (git's rule: ignore rules apply to untracked paths only) every path of the
    /// head and merged trees still in the folder. Files whose stat matches the cache keep their
    /// blob; the others are hashed into `batch`.
    fn scan(&self, batch: &mut Batch) -> io::Result<(Tree, Index)> {
        let cache = self.load_index();
        let mut tree = Tree::new();
        let mut index = Index::new();
        let walk = ignore::WalkBuilder::new(&self.root)
            .hidden(false)
            .parents(false)
            .git_global(false)
            .git_exclude(false)
            .require_git(false)
            .follow_links(false)
            .filter_entry(|entry| {
                !entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| SKIPPED.contains(&name))
            })
            .build();
        for found in walk {
            let found = found.map_err(io::Error::other)?;
            if found.file_type().is_none_or(|kind| kind.is_dir()) {
                continue;
            }
            let rel = found
                .path()
                .strip_prefix(&self.root)
                .map_err(io::Error::other)?;
            let path = rel
                .to_str()
                .ok_or_else(|| io::Error::other(format!("not UTF-8: {}", rel.display())))?
                .to_string();
            let meta = found.path().symlink_metadata()?;
            self.add(&mut tree, &mut index, &cache, batch, path, &meta)?;
        }
        for parent in self.parents() {
            for path in self.objects.change_tree(&parent)?.into_keys() {
                if tree.contains_key(&path) {
                    continue;
                }
                if let Some(meta) = tracked_on_disk(&self.root, &path)? {
                    self.add(&mut tree, &mut index, &cache, batch, path, &meta)?;
                }
            }
        }
        Ok((tree, index))
    }

    /// Put the file at `path` (regular file or symlink; anything else is skipped) into `tree`.
    fn add(
        &self,
        tree: &mut Tree,
        index: &mut Index,
        cache: &Index,
        batch: &mut Batch,
        path: String,
        meta: &fs::Metadata,
    ) -> io::Result<()> {
        let file = self.root.join(&path);
        if let Some((entry, cached)) = read_entry(&file, meta, cache.get(&path), batch)? {
            if let Some(cached) = cached {
                index.insert(path.clone(), cached);
            }
            tree.insert(path, entry);
        }
        Ok(())
    }

    fn load_index(&self) -> Index {
        // A missing or unreadable cache only costs rehashing.
        fs::read(self.dot().join("index.json"))
            .ok()
            .and_then(|text| serde_json::from_slice(&text).ok())
            .unwrap_or_default()
    }

    fn save_index(&self, mut index: Index) -> io::Result<()> {
        let recent = nanos(SystemTime::now().checked_sub(RACY).unwrap_or(UNIX_EPOCH));
        index.retain(|_, cached| cached.mtime < recent && cached.ctime < recent);
        let text = serde_json::to_vec(&index).map_err(io::Error::other)?;
        write_atomic(&self.dot().join("index.json"), &text)
    }
}

/// The entry of `file` (stat `meta`): a symlink, or a regular file, whose blob comes from `hit`
/// when the stat still matches it, else from hashing; with the regular file's new cache line.
/// `None` for anything else (a FIFO, a socket, a device, a folder): reading those can block.
fn read_entry(
    file: &Path,
    meta: &fs::Metadata,
    hit: Option<&Cached>,
    batch: &mut Batch,
) -> io::Result<Option<(Entry, Option<Cached>)>> {
    let kind = meta.file_type();
    if kind.is_symlink() {
        let target = fs::read_link(file)?;
        let entry = Entry {
            blob: batch.put(target.as_os_str().as_bytes())?,
            exec: false,
            link: true,
        };
        return Ok(Some((entry, None)));
    }
    if !kind.is_file() {
        return Ok(None);
    }
    let stat = |blob: Id| Cached {
        size: meta.len(),
        mtime: nanos(meta.modified().unwrap_or(UNIX_EPOCH)),
        ctime: u64::try_from(
            i128::from(meta.ctime()) * 1_000_000_000 + i128::from(meta.ctime_nsec()),
        )
        .unwrap_or(0),
        ino: meta.ino(),
        blob,
    };
    let blob = match hit {
        Some(hit) if *hit == stat(hit.blob.clone()) => hit.blob.clone(),
        _ => batch.put(&fs::read(file)?)?,
    };
    let entry = Entry {
        blob: blob.clone(),
        exec: meta.permissions().mode() & 0o111 != 0,
        link: false,
    };
    Ok(Some((entry, Some(stat(blob)))))
}

/// The stat of the tracked `path` when it is still in the folder: not behind a symlinked or
/// skipped folder.
fn tracked_on_disk(root: &Path, path: &str) -> io::Result<Option<fs::Metadata>> {
    let mut dir = root.to_path_buf();
    let mut parts = path.split('/').peekable();
    while let Some(part) = parts.next() {
        if SKIPPED.contains(&part) {
            return Ok(None);
        }
        dir.push(part);
        let meta = match dir.symlink_metadata() {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        if parts.peek().is_none() {
            return Ok(Some(meta));
        }
        if !meta.is_dir() {
            return Ok(None);
        }
    }
    Ok(None)
}

/// Delete the folder `dir` and everything in it; absent is fine.
fn remove_dir(dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn nanos(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Write `entry` at `root/path`, replacing a file or symlink there. A folder in the way is an error.
pub fn write_entry(objects: &Objects, root: &Path, path: &str, entry: &Entry) -> io::Result<()> {
    let file = root.join(path);
    match file.symlink_metadata() {
        Ok(meta) if meta.is_dir() => {
            return Err(io::Error::other(format!(
                "cannot write {path}: a folder is in the way"
            )));
        }
        Ok(_) => fs::remove_file(&file)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = objects.get(&entry.blob)?;
    if entry.link {
        std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(&bytes), &file)
    } else {
        fs::write(&file, bytes)?;
        set_exec(&file, entry.exec)
    }
}

pub fn set_exec(file: &Path, exec: bool) -> io::Result<()> {
    fs::set_permissions(
        file,
        fs::Permissions::from_mode(if exec { 0o755 } else { 0o644 }),
    )
}

/// Delete `root/path` and then its parent folders that became empty.
pub fn remove_path(root: &Path, path: &str) -> io::Result<()> {
    let file = root.join(path);
    match fs::remove_file(&file) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut dir = file.parent();
    while let Some(current) = dir {
        if current == root || fs::remove_dir(current).is_err() {
            break;
        }
        dir = current.parent();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::scratch;

    type T = io::Result<()>;

    /// A folder's tracked files. The blob ids come from hashing its files, so equal trees mean
    /// equal bytes, exec bits and symlinks.
    fn files(root: &Path) -> io::Result<Tree> {
        Worktree::open(root)?.snapshot()
    }

    #[test]
    fn snapshot_then_materialize_into_an_empty_folder_gives_the_same_files() -> T {
        let root = scratch("worktree-roundtrip")?;
        let (main, copy) = (root.join("main"), root.join("copy"));
        let mut worktree = Worktree::create(&main, "main", None)?;
        fs::create_dir_all(main.join("src/deep"))?;
        fs::write(main.join("src/deep/lib.rs"), "fn f() {}\n")?;
        fs::write(main.join("run.sh"), "#!/bin/sh\n")?;
        set_exec(&main.join("run.sh"), true)?;
        fs::write(main.join(".gitignore"), "build/\n")?;
        fs::create_dir_all(main.join("build"))?;
        fs::write(main.join("build/out"), "ignored")?;
        fs::create_dir_all(main.join(".git"))?;
        fs::write(main.join(".git/HEAD"), "skipped")?;
        std::os::unix::fs::symlink("src/deep/lib.rs", main.join("link"))?;
        let change = worktree.commit("main", "base")?;

        let mut other = Worktree::create(&copy, "copy", Some(main.join(".concord")))?;
        other.materialize(&change)?;
        let tracked = files(&main)?;
        let names: Vec<&str> = tracked.keys().map(String::as_str).collect();
        assert_eq!(names, [".gitignore", "link", "run.sh", "src/deep/lib.rs"]);
        assert_eq!(files(&copy)?, tracked);
        assert!(!copy.join("build").exists());

        // Materializing a later change: deleted paths go, new ones come, ignored files stay.
        fs::remove_dir_all(copy.join("src"))?;
        fs::write(copy.join("new.txt"), "x")?;
        let later = other.commit("copy", "later")?;
        assert!(other.objects().is_ancestor(&change, &later)?);
        worktree.materialize(&later)?;
        assert_eq!(fs::read_to_string(main.join("new.txt"))?, "x");
        assert!(!main.join("src").exists());
        assert!(main.join("build/out").exists());
        Ok(())
    }

    #[test]
    fn merge_in_combines_edits_to_different_lines_and_files() -> T {
        let root = scratch("worktree-merge")?;
        let (a, b) = (root.join("a"), root.join("b"));
        let mut one = Worktree::create(&a, "a", None)?;
        fs::write(a.join("f.txt"), "1\n2\n3\n4\n5\n")?;
        let base = one.commit("a", "base")?;
        let mut two = Worktree::create(&b, "b", Some(a.join(".concord")))?;
        two.materialize(&base)?;

        fs::write(a.join("f.txt"), "one\n2\n3\n4\n5\n")?;
        fs::write(a.join("a.txt"), "from a\n")?;
        one.commit("a", "a's work")?;
        fs::write(b.join("f.txt"), "1\n2\n3\n4\nfive\n")?;
        fs::write(b.join("b.txt"), "from b\n")?;
        let theirs = two.commit("b", "b's work")?;

        assert_eq!(one.merge_in(std::slice::from_ref(&theirs))?, vec![]);
        assert_eq!(fs::read_to_string(a.join("f.txt"))?, "one\n2\n3\n4\nfive\n");
        assert_eq!(fs::read_to_string(a.join("a.txt"))?, "from a\n");
        assert_eq!(fs::read_to_string(a.join("b.txt"))?, "from b\n");
        assert!(one.has_work()?);
        let merged = one.commit("a", "merge")?;
        assert_eq!(one.objects().change(&merged)?.parents.len(), 2);
        assert!(!one.has_work()?);
        // Merging it again changes nothing.
        assert_eq!(one.merge_in(&[theirs])?, vec![]);
        assert!(!one.has_work()?);
        Ok(())
    }

    #[test]
    fn the_stat_cache_sees_a_rewrite_that_keeps_size_and_mtime() -> T {
        let root = scratch("worktree-stat-cache")?;
        let worktree = Worktree::create(&root, "main", None)?;
        let file = root.join("f.txt");
        let old = SystemTime::now() - Duration::from_secs(3600);
        let write = |text: &str| -> T {
            fs::write(&file, text)?;
            fs::File::options().write(true).open(&file)?.set_modified(old)
        };
        write("aaaa")?;
        worktree.snapshot()?;
        write("bbbb")?;
        let blob = &worktree.snapshot()?["f.txt"].blob;
        assert_eq!(worktree.objects().get(blob)?, b"bbbb");
        Ok(())
    }

    #[test]
    fn a_tracked_file_stays_tracked_after_it_is_ignored() -> T {
        let root = scratch("worktree-ignored-later")?;
        let (main, ws) = (root.join("main"), root.join("ws"));
        let mut folder = Worktree::create(&main, "main", None)?;
        fs::write(main.join(".env"), "KEY=1\n")?;
        let base = folder.commit("main", "base")?;
        let mut agent = Worktree::create(&ws, "a", Some(main.join(".concord")))?;
        agent.materialize(&base)?;
        fs::write(ws.join(".gitignore"), ".env\n")?;
        let round = agent.commit("a", "ignore .env")?;
        assert!(agent.objects().change_tree(&round)?.contains_key(".env"));

        assert_eq!(folder.pull(&base, &round)?, vec![]);
        assert_eq!(fs::read_to_string(main.join(".env"))?, "KEY=1\n");
        Ok(())
    }

    #[test]
    fn a_fifo_in_the_folder_is_skipped() -> T {
        let root = scratch("worktree-fifo")?;
        let worktree = Worktree::create(&root, "main", None)?;
        fs::write(root.join("f.txt"), "x")?;
        let made = std::process::Command::new("mkfifo").arg(root.join("pipe")).status()?;
        assert!(made.success());
        let names: Vec<String> = worktree.snapshot()?.into_keys().collect();
        assert_eq!(names, ["f.txt"]);
        Ok(())
    }

    #[test]
    fn a_criss_cross_merge_keeps_a_deletion() -> T {
        let root = scratch("worktree-criss-cross")?;
        let (a, b) = (root.join("a"), root.join("b"));
        let mut one = Worktree::create(&a, "a", None)?;
        fs::write(a.join("f.txt"), "f\n")?;
        let base = one.commit("a", "base")?;
        let mut two = Worktree::create(&b, "b", Some(a.join(".concord")))?;
        two.materialize(&base)?;

        fs::write(a.join("a.txt"), "a\n")?;
        let a0 = one.commit("a", "a0")?;
        fs::write(b.join("g.txt"), "g\n")?;
        let b1 = two.commit("b", "b1 adds g")?;
        fs::write(b.join("h.txt"), "h\n")?;
        two.commit("b", "b1x")?;

        // a takes b1 and then deletes g; b takes a0: a criss-cross.
        assert_eq!(one.merge_in(&[b1])?, vec![]);
        one.commit("a", "a1")?;
        fs::remove_file(a.join("g.txt"))?;
        one.commit("a", "a2 deletes g")?;
        assert_eq!(two.merge_in(&[a0])?, vec![]);
        let b2 = two.commit("b", "b2")?;

        assert_eq!(one.merge_in(&[b2])?, vec![]);
        assert!(!a.join("g.txt").exists());
        assert_eq!(fs::read_to_string(a.join("h.txt"))?, "h\n");
        Ok(())
    }
}
