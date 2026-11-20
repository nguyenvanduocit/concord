//! The project's folders: the main folder (its `.concord` holds the store and the board), one
//! workspace per agent next to it, created at the board's base and removed after landing.

use std::fs;
use std::path::{Path, PathBuf};

use crate::core::Landing;
use crate::store;
use crate::worktree::Worktree;

/// Name of the main folder's workspace state; never an agent's (agents' workspaces name a store).
const MAIN: &str = "main";

/// Where an agent's workspace lives: a sibling of the main folder, `<main>-<agent>`.
/// Siblings, not subdirectories: tools that walk up (cargo workspaces, git) must not see the main
/// project from inside an agent workspace.
pub fn workspace_path(main_root: &Path, agent: &str) -> PathBuf {
    let name = main_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".to_string());
    main_root.with_file_name(format!("{name}-{agent}"))
}

/// The project's store: `.concord` in the main folder.
pub fn store_dir(main_root: &Path) -> PathBuf {
    main_root.join(".concord")
}

/// The main folder of the project `dir` belongs to. With none, the project starts in the nearest
/// folder holding `.git` (only its presence is read), else in `dir`.
pub fn prepare(dir: &Path) -> Result<PathBuf, String> {
    let dir = fs::canonicalize(dir).map_err(|e| e.to_string())?;
    if let Ok(place) = store::locate(&dir) {
        return place
            .board
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| "the project's store has no folder".to_string());
    }
    let root = dir
        .ancestors()
        .find(|d| d.join(".git").exists())
        .unwrap_or(&dir)
        .to_path_buf();
    Worktree::create(&root, MAIN, None).map_err(|e| e.to_string())?;
    Ok(root)
}

/// Where agents start: the main folder as it is now, uncommitted edits included, as a change (the
/// folder's last change when nothing changed since).
pub fn landing(main_root: &Path) -> Result<Landing, String> {
    let fail = |e: std::io::Error| format!("snapshot of {}: {e}", main_root.display());
    let mut main = Worktree::open(main_root).map_err(fail)?;
    let base = match &main.state().head {
        Some(head) if !main.has_work().map_err(fail)? => head.clone(),
        _ => main.commit(MAIN, "concord: base").map_err(fail)?,
    };
    Ok(Landing { base })
}

/// One fresh workspace per agent starting at `base`; workspaces of `previous` agents are removed
/// first (their work kept, see `remove`). Returns `(agent, path)` pairs.
pub fn create(
    main_root: &Path,
    team: &[String],
    previous: &[String],
    base: &str,
) -> Result<Vec<(String, PathBuf)>, String> {
    let all: Vec<String> = previous.iter().chain(team).cloned().collect();
    remove(main_root, &all, base)?;
    let mut created = Vec::new();
    for agent in team {
        let path = workspace_path(main_root, agent);
        if path.exists() {
            return Err(format!(
                "{} exists and is not a workspace of this project; move it away",
                path.display()
            ));
        }
        let fail = |e: std::io::Error| format!("workspace {}: {e}", path.display());
        let mut workspace =
            Worktree::create(&path, agent, Some(store_dir(main_root))).map_err(fail)?;
        workspace.materialize(base).map_err(fail)?;
        created.push((agent.clone(), path));
    }
    Ok(created)
}

/// Remove the agents' workspaces, each after `keep` against `base` (the base the project moves
/// to). Returns the agents whose workspace was removed.
pub fn remove(main_root: &Path, team: &[String], base: &str) -> Result<Vec<String>, String> {
    let mut removed = Vec::new();
    for agent in team {
        let path = workspace_path(main_root, agent);
        let Some(mut workspace) = ours(main_root, &path) else {
            continue;
        };
        keep(&mut workspace, agent, base)
            .map_err(|e| format!("workspace {}: {e}", path.display()))?;
        fs::remove_dir_all(&path).map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
        removed.push(agent.clone());
    }
    Ok(removed)
}

/// Before a folder's files are replaced or removed: files not in a change yet become one, and an
/// agent head `base` does not hold is printed (`KEPT <agent>'s work as change <id>`), so work that
/// never landed can be found again.
pub fn keep(folder: &mut Worktree, agent: &str, base: &str) -> std::io::Result<()> {
    let Some(head) = folder.state().head.clone() else {
        return Ok(());
    };
    let head = if folder.has_work()? {
        folder.commit(agent, &format!("{agent}: left in the workspace"))?
    } else {
        head
    };
    if !folder.objects().is_ancestor(&head, base)? {
        out!("KEPT {agent}'s work as change {}", crate::objects::short(&head));
    }
    Ok(())
}

/// Remove the folder `path` if it is a workspace of this project, whatever it holds.
pub fn discard(main_root: &Path, path: &Path) -> Result<(), String> {
    if ours(main_root, path).is_some() {
        fs::remove_dir_all(path).map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
    }
    Ok(())
}

/// The workspace at `path` when it belongs to this project's store.
fn ours(main_root: &Path, path: &Path) -> Option<Worktree> {
    let workspace = Worktree::open(path).ok()?;
    (workspace.state().store.as_deref() == Some(store_dir(main_root).as_path())).then_some(workspace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspaces_are_siblings_named_after_the_project() {
        let main = Path::new("/work/shop");
        assert_eq!(
            workspace_path(main, "agent-a"),
            Path::new("/work/shop-agent-a")
        );
    }
}
