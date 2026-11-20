//! `concord serve`: the project's board and objects over HTTP, for agents on other machines.
//! Every URL starts with `/t/<token>`; the token names the agent.
//!   GET  /t/<token>/api/whoami       project, agent, base change (used by `concord join`)
//!   GET  /t/<token>/api/board        the board (status, log, dry runs)
//!   POST /t/<token>/api/command      one board command, applied like a local one
//!   POST /t/<token>/api/missing      object ids → the ones the server lacks
//!   POST /t/<token>/api/objects      store objects (`remote::encode`), all durable at once;
//!                                    refused unless every object's bytes hash to its id
//!   POST /t/<token>/api/objects/get  object ids → those objects (`remote::encode`)

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};
use std::thread;
use std::time::Duration;

use tiny_http::{Header, Method, Request, Response, Server};

use crate::cli::Failure;
use crate::core::{Board, Command, Outcome};
use crate::objects::{Id, Objects, short};
use crate::remote::{self, Reply, Whoami};
use crate::store::{Store, write_atomic};

pub const DEFAULT_PORT: u16 = 7457;
const TOKENS: &str = "tokens.json";
const SERVING: &str = "serving.json";

/// One random token per agent (made by `init`, replacing the previous team's); kept in
/// `tokens.json` beside the board, never in it.
pub fn issue_tokens(board_dir: &Path, team: &[String]) -> Result<(), String> {
    let mut fresh = BTreeMap::new();
    for agent in team {
        fresh.insert(agent.clone(), new_token()?);
    }
    update_tokens(board_dir, |tokens| *tokens = fresh)
}

/// A token for an agent added to the running team; the others keep theirs. A running server
/// reads tokens on every request, so it accepts the new one at once.
pub fn add_token(board_dir: &Path, agent: &str) -> Result<String, String> {
    let token = new_token()?;
    update_tokens(board_dir, |tokens| {
        tokens.insert(agent.to_string(), token.clone());
    })?;
    Ok(token)
}

/// Read-modify-write `tokens.json` under its own `flock` (two `add`s at once both land), written
/// atomically (a server reading it meanwhile sees the old or the new file).
fn update_tokens(
    board_dir: &Path,
    change: impl FnOnce(&mut BTreeMap<String, String>),
) -> Result<(), String> {
    let fail = |e: io::Error| format!("cannot write {TOKENS}: {e}");
    fs::create_dir_all(board_dir).map_err(fail)?;
    let lock = File::create(board_dir.join("tokens.lock")).map_err(fail)?;
    lock.lock().map_err(fail)?;
    let mut tokens = tokens(board_dir);
    change(&mut tokens);
    let text = serde_json::to_vec_pretty(&tokens).map_err(|e| e.to_string())?;
    write_atomic(&board_dir.join(TOKENS), &text).map_err(fail)
    // `lock` drops here, after the new file is in place.
}

fn new_token() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| format!("no randomness: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Base URL of this board's server (`http://host:port`), written by `serve` when it starts;
/// `None` unless that server answers now.
pub fn live_url(board_dir: &Path) -> Option<String> {
    let url = fs::read_to_string(board_dir.join(SERVING)).ok()?;
    let url = url.trim().to_string();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(2)))
        .http_status_as_error(false)
        .build()
        .into();
    agent.get(format!("{url}/")).call().ok().map(|_| url)
}

fn tokens(board_dir: &Path) -> BTreeMap<String, String> {
    fs::read_to_string(board_dir.join(TOKENS))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Serve until killed. `root`: main folder; `board_dir`: its `.concord`.
pub fn run(root: &Path, board_dir: &Path, port: u16) -> Result<(), String> {
    let server = Server::http(("0.0.0.0", port))
        .map_err(|e| format!("cannot listen on port {port}: {e}"))?;
    let board = Store::new(board_dir.to_path_buf())
        .load()
        .map_err(|e| format!("board I/O: {e}"))?
        .ok_or("no board; run concord init first")?;
    let host = host();
    let tokens = tokens(board_dir);
    fs::write(board_dir.join(SERVING), format!("http://{host}:{port}\n"))
        .map_err(|e| format!("cannot write {SERVING}: {e}"))?;
    out!("SERVING {} on port {port}", root.display());
    for agent in &board.team {
        if let Some(token) = tokens.get(agent) {
            out!("JOIN {agent} concord join http://{host}:{port}/t/{token}");
        }
    }
    let _ = io::stdout().flush();
    let ctx = Context {
        root: root.to_path_buf(),
        board_dir: board_dir.to_path_buf(),
    };
    for request in server.incoming_requests() {
        let ctx = ctx.clone();
        let spawned = thread::Builder::new().spawn(move || handle(&ctx, request));
        if let Err(e) = spawned {
            err_out!("cannot handle a request: {e}");
        }
    }
    Ok(())
}

#[derive(Clone)]
struct Context {
    root: PathBuf,
    board_dir: PathBuf,
}

fn handle(ctx: &Context, mut request: Request) {
    let url = request.url().to_string();
    let path = url.split_once('?').map_or(url.as_str(), |(path, _)| path);
    let store = Store::new(ctx.board_dir.clone());
    let board = match store.load() {
        Ok(Some(board)) => board,
        _ => return respond(request, text(503, "no board")),
    };
    let Some((agent, rest)) = authorize(&tokens(&ctx.board_dir), path) else {
        return respond(request, text(403, "unknown token"));
    };
    let objects = Objects::new(&ctx.board_dir);
    let method = request.method().clone();
    let response = match (method, rest) {
        (Method::Get, "api/whoami") => whoami(ctx, &board, &agent),
        (Method::Get, "api/board") => json(&board),
        (Method::Post, "api/command") => command(&store, &objects, &board, &agent, &mut request),
        (Method::Post, "api/missing") => missing(&objects, &mut request),
        (Method::Post, "api/objects") => put_objects(&objects, &mut request),
        (Method::Post, "api/objects/get") => get_objects(&objects, &mut request),
        _ => text(404, "not found"),
    };
    respond(request, response);
}

fn missing(objects: &Objects, request: &mut Request) -> Response<io::Cursor<Vec<u8>>> {
    let ids: Vec<Id> = match serde_json::from_reader(request.as_reader()) {
        Ok(ids) => ids,
        Err(e) => return text(400, &format!("bad id list: {e}")),
    };
    let lacking: Vec<&Id> = ids.iter().filter(|id| !objects.has(id)).collect();
    json(&lacking)
}

/// Store every object of the body in one batch: one flush of the drive for the whole upload.
fn put_objects(objects: &Objects, request: &mut Request) -> Response<io::Cursor<Vec<u8>>> {
    let mut body = Vec::new();
    if request.as_reader().read_to_end(&mut body).is_err() {
        return text(400, "unreadable body");
    }
    let sent = match remote::decode(&body) {
        Ok(sent) => sent,
        Err(e) => return text(400, &e),
    };
    let mut batch = objects.batch();
    let stored = sent
        .iter()
        .try_for_each(|(_, bytes)| batch.put(bytes).map(drop))
        .and_then(|()| batch.finish());
    match stored {
        Ok(()) => text(200, &format!("stored {}", sent.len())),
        Err(e) => text(500, &e.to_string()),
    }
}

fn get_objects(objects: &Objects, request: &mut Request) -> Response<io::Cursor<Vec<u8>>> {
    let ids: Vec<Id> = match serde_json::from_reader(request.as_reader()) {
        Ok(ids) => ids,
        Err(e) => return text(400, &format!("bad id list: {e}")),
    };
    let found: Result<Vec<(Id, Vec<u8>)>, io::Error> = ids
        .into_iter()
        .map(|id| objects.get(&id).map(|bytes| (id, bytes)))
        .collect();
    match found {
        Ok(found) => Response::from_data(remote::encode(&found)),
        Err(e) => text(404, &e.to_string()),
    }
}

/// `/t/<token>/<rest>` with a known token: the agent it belongs to and `<rest>`.
fn authorize<'a>(tokens: &BTreeMap<String, String>, path: &'a str) -> Option<(String, &'a str)> {
    let rest = path.strip_prefix("/t/")?;
    let (token, rest) = rest.split_once('/').unwrap_or((rest, ""));
    let agent = tokens
        .iter()
        .find(|(_, t)| constant_time_eq(t.as_bytes(), token.as_bytes()))
        .map(|(agent, _)| agent.clone())?;
    Some((agent, rest))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn whoami(ctx: &Context, board: &Board, agent: &str) -> Response<io::Cursor<Vec<u8>>> {
    let Some(landing) = &board.landing else {
        return text(503, "board has no base; run concord init");
    };
    let base = landing.base.clone();
    let project = ctx
        .root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".to_string());
    json(&Whoami {
        agent: agent.to_string(),
        project,
        base,
    })
}

fn command(
    store: &Store,
    objects: &Objects,
    board: &Board,
    agent: &str,
    request: &mut Request,
) -> Response<io::Cursor<Vec<u8>>> {
    let mut body = String::new();
    if request.as_reader().read_to_string(&mut body).is_err() {
        return text(400, "unreadable body");
    }
    let cmd: Command = match serde_json::from_str(&body) {
        Ok(cmd) => cmd,
        Err(e) => return text(400, &format!("bad command: {e}")),
    };
    // The board may name only changes everyone can read: the gate combines them here.
    if let Command::Done { change, .. } | Command::Finish { change, .. } = &cmd
        && !objects.complete(change, &complete_on(board)).unwrap_or(false)
    {
        return json(&Reply::Err {
            code: crate::cli::EXIT_SETUP,
            message: format!(
                "ERROR change {} is not complete on the server; run the command again",
                short(change)
            ),
        });
    }
    let reply = match command_agent(&cmd) {
        Some(owner) if owner == agent => match apply(store, &cmd, agent) {
            Ok(outcome) => Reply::Ok(outcome),
            Err(f) => Reply::Err {
                code: f.code,
                message: f.message,
            },
        },
        _ => Reply::Err {
            code: crate::cli::EXIT_ERROR,
            message: format!("ERROR this token belongs to {agent}"),
        },
    };
    json(&reply)
}

/// Apply a remote agent's command as a local one would be. Everything that touches the project's
/// folders (combine, land, discard) runs here, on the serving machine; a landing that fails is the
/// command's failure, so the remote `land` exits as a local one would.
fn apply(store: &Store, cmd: &Command, agent: &str) -> Result<Outcome, Failure> {
    if let Command::Approve { .. } = cmd {
        return crate::cli::approve(store, cmd);
    }
    let outcome = crate::cli::execute(store, cmd)?;
    match &outcome {
        Outcome::Finished { unseen, .. } if unseen.is_empty() => {
            crate::cli::start_review_if_due(store, agent);
        }
        Outcome::Rejected { .. } => crate::cli::after_reject(store, &outcome),
        _ => {}
    }
    Ok(outcome)
}

/// Changes the board names were checked complete when it recorded them (or were made here): a
/// completeness walk stops at them.
fn complete_on(board: &Board) -> BTreeSet<Id> {
    let mut known: BTreeSet<Id> = board.heads.values().cloned().collect();
    known.extend(board.history.values().flatten().map(|c| c.change.clone()));
    known.extend(board.landing.as_ref().map(|l| l.base.clone()));
    known.extend(board.review.as_ref().map(|r| r.combined.clone()));
    known
}

/// The agent a worker command acts for; `None` for orchestrator commands.
fn command_agent(cmd: &Command) -> Option<&str> {
    match cmd {
        // Joining and landing happen on the server's machine, never through a token.
        Command::Init { .. }
        | Command::Release { .. }
        | Command::Add { .. }
        | Command::Join { .. }
        | Command::StartReview { .. }
        | Command::Landed { .. }
        | Command::GateFailed { .. } => None,
        Command::Plan { agent, .. }
        | Command::Claim { agent, .. }
        | Command::Done { agent, .. }
        | Command::Finish { agent, .. }
        | Command::Approve { agent, .. }
        | Command::Reject { agent, .. } => Some(agent),
    }
}

/// The address other machines reach this one at: its Tailscale IPv4, else its hostname.
fn host() -> String {
    for tailscale in [
        "tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    ] {
        if let Some(ip) = first_line(Process::new(tailscale).args(["ip", "-4"])) {
            return ip;
        }
    }
    first_line(&mut Process::new("hostname")).unwrap_or_else(|| "localhost".to_string())
}

fn first_line(cmd: &mut Process) -> Option<String> {
    let out = cmd.stderr(Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().next()?.trim().to_string();
    (out.status.success() && !line.is_empty()).then_some(line)
}

fn json<T: serde::Serialize>(value: &T) -> Response<io::Cursor<Vec<u8>>> {
    match serde_json::to_vec(value) {
        Ok(body) => typed(Response::from_data(body), "application/json"),
        Err(e) => text(500, &e.to_string()),
    }
}

fn text(status: u16, message: &str) -> Response<io::Cursor<Vec<u8>>> {
    typed(
        Response::from_string(message).with_status_code(status),
        "text/plain",
    )
}

fn typed<R: Read>(response: Response<R>, value: &str) -> Response<R> {
    match Header::from_bytes(&b"Content-Type"[..], value.as_bytes()) {
        Ok(header) => response.with_header(header),
        Err(()) => response,
    }
}

fn respond<R: Read>(request: Request, response: Response<R>) {
    let _ = request.respond(response);
}
