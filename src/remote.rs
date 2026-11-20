//! Client side of `concord serve`: a clone made by `concord join` sends its worker commands to
//! the server recorded in `remote.json` (in the clone's `.concord`), and moves objects to and from
//! it by hash. Wire types live here too.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::core::{Board, Command, Outcome};
use crate::objects::{Change, Id, Objects, Tree, hash, short};

const FILE: &str = "remote.json";

/// Server answer to a board command: the outcome, or the error a local run would have exited with.
#[derive(Serialize, Deserialize)]
pub enum Reply {
    Ok(Outcome),
    Err { code: u8, message: String },
}

/// What a join needs to know about the project behind a URL.
#[derive(Serialize, Deserialize)]
pub struct Whoami {
    pub agent: String,
    pub project: String,
    /// Change id the agents start from.
    pub base: String,
}

/// The server a joined clone talks to: `url` already carries the agent's token.
#[derive(Serialize, Deserialize)]
pub struct Remote {
    pub url: String,
    pub agent: String,
}

pub fn load(board_dir: &Path) -> Option<Remote> {
    let text = fs::read_to_string(board_dir.join(FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save(board_dir: &Path, remote: &Remote) -> Result<(), String> {
    let text = serde_json::to_string_pretty(remote).map_err(|e| e.to_string())?;
    fs::create_dir_all(board_dir).map_err(|e| e.to_string())?;
    fs::write(board_dir.join(FILE), text).map_err(|e| format!("cannot write {FILE}: {e}"))
}

pub fn whoami(url: &str) -> Result<Whoami, String> {
    get_json(url, "api/whoami")
}

/// The agent token of a join URL (`.../t/<token>`): the same token is the same membership,
/// whatever host the URL names.
pub fn token(url: &str) -> Option<&str> {
    url.rsplit_once("/t/").map(|(_, token)| token.trim_end_matches('/'))
}

/// Length of a full object id on the wire.
const ID_LEN: usize = 64;

/// Objects on the wire (`api/objects`, `api/objects/get`): per object its id (64 hex digits), the
/// length of its bytes (8 bytes, big endian), its bytes.
pub fn encode(objects: &[(Id, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (id, bytes) in objects {
        out.extend_from_slice(id.as_bytes());
        out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
        out.extend_from_slice(bytes);
    }
    out
}

/// The objects of `encode`'s output, each checked against its id.
pub fn decode(mut wire: &[u8]) -> Result<Vec<(Id, Vec<u8>)>, String> {
    let cut = "objects cut short";
    let mut objects = Vec::new();
    while !wire.is_empty() {
        let (id, rest) = wire.split_at_checked(ID_LEN).ok_or(cut)?;
        let (len, rest) = rest.split_at_checked(8).ok_or(cut)?;
        let len = u64::from_be_bytes(len.try_into().map_err(|_| cut)?);
        let len = usize::try_from(len).map_err(|_| cut)?;
        let (bytes, rest) = rest.split_at_checked(len).ok_or(cut)?;
        let id = String::from_utf8_lossy(id).into_owned();
        if hash(bytes) != id {
            return Err(format!("object {} does not hash to its id", short(&id)));
        }
        objects.push((id, bytes.to_vec()));
        wire = rest;
    }
    Ok(objects)
}

impl Remote {
    /// Apply a command on the server's board; blocks as long as the command waits there.
    pub fn command(&self, cmd: &Command) -> Result<Reply, String> {
        let mut response = agent()
            .post(endpoint(&self.url, "api/command"))
            .send_json(cmd)
            .map_err(|e| unreachable(&self.url, e))?;
        response
            .body_mut()
            .read_json()
            .map_err(|e| format!("bad answer from {}: {e}", self.url))
    }

    pub fn board(&self) -> Result<Option<Board>, String> {
        get_json(&self.url, "api/board")
    }

    /// Send the server every object of `change` it lacks: the change and its ancestors, their
    /// trees and blobs. Uploaded blobs first, then trees, then changes parents first, so the
    /// server never holds an object pointing at one it lacks.
    pub fn push(&self, objects: &Objects, change: &str) -> Result<(), String> {
        let io = |e: std::io::Error| e.to_string();
        let mut changes = Vec::new();
        let mut frontier = vec![change.to_string()];
        let mut seen = BTreeSet::new();
        while !frontier.is_empty() {
            frontier.retain(|id| seen.insert(id.clone()));
            let missing = self.missing(&frontier)?;
            frontier.clear();
            for id in missing {
                frontier.extend(objects.change(&id).map_err(io)?.parents);
                changes.push(id);
            }
        }
        let trees: Vec<Id> = changes
            .iter()
            .map(|id| objects.change(id).map(|c| c.tree))
            .collect::<Result<_, _>>()
            .map_err(io)?;
        let trees = self.missing(&trees)?;
        let mut blobs = BTreeSet::new();
        for tree in &trees {
            blobs.extend(objects.tree(tree).map_err(io)?.into_values().map(|e| e.blob));
        }
        let blobs = self.missing(&blobs.into_iter().collect::<Vec<_>>())?;
        let upload: Vec<(Id, Vec<u8>)> = blobs
            .iter()
            .chain(&trees)
            .chain(changes.iter().rev())
            .map(|id| objects.get(id).map(|bytes| (id.clone(), bytes)))
            .collect::<Result<_, _>>()
            .map_err(io)?;
        if upload.is_empty() {
            return Ok(());
        }
        agent()
            .post(endpoint(&self.url, "api/objects"))
            .send(&encode(&upload)[..])
            .map_err(|e| unreachable(&self.url, e))?;
        Ok(())
    }

    /// Get every object of `change` this clone lacks from the server: one request per generation
    /// of missing changes (with their trees), one for all the blobs. Written blobs first, then
    /// trees, then changes parents first, so the clone never holds an object pointing at one it
    /// lacks.
    pub fn fetch(&self, objects: &Objects, change: &str) -> Result<(), String> {
        let io = |e: std::io::Error| e.to_string();
        let parse = |what: &str, id: &str, bytes: &[u8]| {
            format!("bad {what} {} from the server: {}", short(id), String::from_utf8_lossy(bytes))
        };
        let mut changes: Vec<(Id, Vec<u8>)> = Vec::new();
        let mut trees: Vec<(Id, Vec<u8>)> = Vec::new();
        let mut blobs = BTreeSet::new();
        let mut todo = vec![change.to_string()];
        let mut seen = BTreeSet::new();
        while !todo.is_empty() {
            todo.retain(|id| !objects.has(id) && seen.insert(id.clone()));
            let mut tree_ids = Vec::new();
            for (id, bytes) in self.get(&std::mem::take(&mut todo))? {
                let found: Change =
                    serde_json::from_slice(&bytes).map_err(|_| parse("change", &id, &bytes))?;
                todo.extend(found.parents);
                if !objects.has(&found.tree) && seen.insert(found.tree.clone()) {
                    tree_ids.push(found.tree);
                }
                changes.push((id, bytes));
            }
            for (id, bytes) in self.get(&tree_ids)? {
                let tree: Tree =
                    serde_json::from_slice(&bytes).map_err(|_| parse("tree", &id, &bytes))?;
                blobs.extend(tree.into_values().map(|e| e.blob).filter(|b| !objects.has(b)));
                trees.push((id, bytes));
            }
        }
        let blobs = self.get(&blobs.into_iter().collect::<Vec<_>>())?;
        let mut batch = objects.batch();
        for (_, bytes) in blobs.iter().chain(&trees).chain(changes.iter().rev()) {
            batch.put(bytes).map_err(io)?;
        }
        batch.finish().map_err(io)
    }

    /// The ids among `ids` the server lacks.
    fn missing(&self, ids: &[Id]) -> Result<Vec<Id>, String> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut response = agent()
            .post(endpoint(&self.url, "api/missing"))
            .send_json(ids)
            .map_err(|e| unreachable(&self.url, e))?;
        response
            .body_mut()
            .with_config()
            .limit(u64::MAX)
            .read_json()
            .map_err(|e| format!("bad answer from {}: {e}", self.url))
    }

    /// The objects `ids`, in one request, each checked against its id.
    fn get(&self, ids: &[Id]) -> Result<Vec<(Id, Vec<u8>)>, String> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut response = agent()
            .post(endpoint(&self.url, "api/objects/get"))
            .send_json(ids)
            .map_err(|e| unreachable(&self.url, e))?;
        let wire = response
            .body_mut()
            .with_config()
            .limit(u64::MAX)
            .read_to_vec()
            .map_err(|e| format!("bad answer from {}: {e}", self.url))?;
        let found = decode(&wire).map_err(|e| format!("bad answer from {}: {e}", self.url))?;
        if found.iter().map(|(id, _)| id).ne(ids.iter()) {
            return Err(format!("{} sent other objects than asked for", self.url));
        }
        Ok(found)
    }
}

fn endpoint(url: &str, path: &str) -> String {
    format!("{}/{path}", url.trim_end_matches('/'))
}

fn get_json<T: serde::de::DeserializeOwned>(url: &str, path: &str) -> Result<T, String> {
    let mut response = agent()
        .get(endpoint(url, path))
        .call()
        .map_err(|e| unreachable(url, e))?;
    response
        .body_mut()
        .with_config()
        .limit(u64::MAX)
        .read_json()
        .map_err(|e| format!("bad answer from {url}: {e}"))
}

/// No timeout: `claim` and `finish` wait on the server as long as they would locally.
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(None)
        .build()
        .into()
}

fn unreachable(url: &str, e: ureq::Error) -> String {
    format!("cannot reach the concord server {url}: {e}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objects_cross_the_wire_whole_and_checked() {
        let objects = vec![(hash(b"a"), b"a".to_vec()), (hash(b""), Vec::new())];
        assert_eq!(decode(&encode(&objects)), Ok(objects.clone()));
        let forged = encode(&[(hash(b"a"), b"b".to_vec())]);
        assert!(decode(&forged).is_err_and(|e| e.contains("does not hash")));
        let wire = encode(&objects);
        assert!(decode(&wire[..wire.len() - 1]).is_err());
    }
}
