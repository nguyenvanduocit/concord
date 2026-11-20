//! Landing gate, mechanical part: combine every finished agent head (Mergiraf per file), refuse
//! lost work and leftover conflicts. The combined result then waits in a checkout of its own for
//! the agents' review (`docs/design.md`, landing); an approved review pulls it into the main
//! folder, a rejected one discards it.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::core::{Board, Review};
use crate::objects::{Objects, short};
use crate::team;
use crate::worktree::Worktree;

/// Folder name suffix of the review checkout; agent names must differ from it.
const GATE_WORKSPACE: &str = "concord-gate";

pub enum Combined {
    Ready { change: String },
    Fail { reason: String },
}

/// Where the combined result is checked out for its reviewers.
pub fn checkout(root: &Path) -> PathBuf {
    team::workspace_path(root, GATE_WORKSPACE)
}

/// `heads`: finished changes from the board. Merges in a folder of its own
/// (`<project>-concord-gate`), so the main folder, where people and agents may be editing, is
/// untouched. On success the folder stays, holding the combined result for the reviewers.
pub fn combine(root: &Path, heads: &[String], base: Option<&str>) -> Result<Combined, String> {
    if heads.is_empty() {
        return Ok(fail("no agent heads on board"));
    }
    let objects = Objects::new(&team::store_dir(root));
    let err = |e: std::io::Error| e.to_string();
    let mut lost = Vec::new();
    for head in heads {
        if lost_work(&objects, head, base).map_err(err)? {
            lost.push(short(head).to_string());
        }
    }
    if !lost.is_empty() {
        return Ok(fail(&format!(
            "agent heads without work of their own (work lost): {}",
            lost.join(" ")
        )));
    }
    let tips = objects.heads(heads).map_err(err)?;
    let shorts = |ids: &[String]| ids.iter().map(|id| short(id)).collect::<Vec<_>>().join(" ");
    out!(
        "agent heads: {}  ->  tips to combine: {}",
        shorts(heads),
        shorts(&tips)
    );
    let Some((first, rest)) = tips.split_first() else {
        return Ok(fail("agent heads not found in the store"));
    };
    // A checkout left by an earlier review or a crash goes first: re-running starts clean.
    discard(root)?;
    team::create(root, &[GATE_WORKSPACE.to_string()], &[], first)?;
    let dir = checkout(root);
    let mut gate = Worktree::open(&dir).map_err(err)?;
    if rest.is_empty() {
        return Ok(Combined::Ready {
            change: first.clone(),
        });
    }
    gate.merge_in(rest).map_err(err)?;
    let left = gate.unresolved().map_err(err)?;
    if !left.is_empty() {
        discard(root)?;
        let paths: Vec<&str> = left.iter().map(|c| c.path.as_str()).collect();
        return Ok(fail(&format!("unresolved conflicts: {}", paths.join(" "))));
    }
    let change = gate
        .commit("concord", &format!("concord: combine {}", shorts(&tips)))
        .map_err(err)?;
    Ok(Combined::Ready { change })
}

/// A finished change brings nothing to the round, judged over the agent's whole contribution
/// since the base: it is the base or an ancestor of it, or its tree is the base's and none of its
/// changes the base lacks is another agent's (merged work that was then undone is not lost).
fn lost_work(objects: &Objects, head: &str, base: Option<&str>) -> std::io::Result<bool> {
    let Some(base) = base else {
        return Ok(false);
    };
    let in_base = objects.ancestors(&[base.to_string()])?;
    if in_base.contains_key(head) {
        return Ok(true);
    }
    let change = objects.change(head)?;
    if change.tree != objects.change(base)?.tree {
        return Ok(false);
    }
    for id in objects.ancestors(&[head.to_string()])?.into_keys() {
        if !in_base.contains_key(&id) && objects.change(&id)?.agent != change.agent {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Approved: pull `combined` into the main folder (`base` → `combined` merged into the files as
/// they are), then drop the review checkout. Prints `LANDED` and any conflicts the pull left.
pub fn land(root: &Path, base: &str, combined: &str) -> Result<(), String> {
    discard(root)?;
    let err = |e: std::io::Error| format!("pulling into {}: {e}", root.display());
    let mut main = Worktree::open(root).map_err(err)?;
    let conflicts = main.pull(base, combined).map_err(err)?;
    out!("LANDED {}", short(combined));
    if !conflicts.is_empty() {
        let paths: Vec<&str> = conflicts.iter().map(|c| c.path.as_str()).collect();
        out!(
            "CONFLICTS {}: the landed round and uncommitted edits in {} both changed these; \
             settle them there",
            paths.join(" "),
            root.display()
        );
    }
    Ok(())
}

/// Drop the review checkout, whatever the reviewers wrote there.
pub fn discard(root: &Path) -> Result<(), String> {
    team::discard(root, &checkout(root))
}

/// The ledger line of a landed round (`ledger.jsonl`): who landed what on which base, their
/// criteria and contracts, and each reviewer's evidence.
pub fn ledger_entry(board: &Board, review: &Review, at: u64) -> serde_json::Value {
    let agents: Vec<serde_json::Value> = review
        .agents
        .iter()
        .map(|agent| {
            let contracts: Vec<_> = board
                .history
                .values()
                .flatten()
                .filter(|c| &c.agent == agent)
                .collect();
            json!({
                "agent": agent,
                "head": board.heads.get(agent),
                "criteria": board.plans.get(agent).map(|p| &p.criteria),
                "contracts": contracts,
            })
        })
        .collect();
    let reviews: Vec<serde_json::Value> = review
        .approved
        .iter()
        .map(|(reviewer, evidence)| json!({"reviewer": reviewer, "evidence": evidence}))
        .collect();
    json!({
        "at": at,
        "base": board.landing.as_ref().map(|l| &l.base),
        "landed": review.combined,
        "agents": agents,
        "reviews": reviews,
    })
}

fn fail(reason: &str) -> Combined {
    Combined::Fail {
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::{Change, Entry, Id, Tree};
    use crate::test_util::scratch;

    type T = std::io::Result<()>;

    #[test]
    fn work_is_lost_only_when_the_agents_whole_contribution_is_empty() -> T {
        let objects = Objects::new(&scratch("gate-lost-work")?);
        let mut batch = objects.batch();
        let mut tree = |text: &[u8]| -> std::io::Result<Id> {
            let blob = batch.put(text)?;
            let entry = Entry { blob, exec: false, link: false };
            batch.put_tree(&Tree::from([("m.py".to_string(), entry)]))
        };
        let (old, new) = (tree(b"1")?, tree(b"10")?);
        let mut change = |tree: &Id, parents: &[&Id], agent: &str| {
            let parents = parents.iter().map(|p| p.to_string()).collect();
            let message = String::new();
            batch.put_change(&Change { tree: tree.clone(), parents, agent: agent.into(), message })
        };
        let base = change(&old, &[], "main")?;
        let work = change(&new, &[&base], "a")?;
        // A second `done` with nothing new on top of the work.
        let empty_on_top = change(&new, &[&work], "a")?;
        let undone = change(&old, &[&work], "a")?;
        let b_work = change(&new, &[&base], "b")?;
        let undone_merge = change(&old, &[&b_work], "a")?;
        batch.finish()?;
        let lost = |head: &Id| lost_work(&objects, head, Some(&base));
        assert!(!lost(&empty_on_top)?, "a change with no edit of its own on top of work");
        assert!(lost(&base)?, "finished at the base");
        assert!(lost(&undone)?, "the agent's own work, undone");
        assert!(!lost(&undone_merge)?, "brings another agent's change the base lacks");
        Ok(())
    }
}
