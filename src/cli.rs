//! The command line: argument parsing, the commands, their output and exit codes.

use std::fs;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::core::{self, Board, Command, Harness, Outcome, Session, Step, list_or_none, py_list};
use crate::merge::Conflict;
use crate::objects::{Objects, short};
use crate::remote::{Remote, Reply};
use crate::store::{self, Store};
use crate::worktree::Worktree;
use crate::{diff, gate, remote, serve, team};

const USAGE: &str = "usage: concord <command> ...   (no setup: run inside the project)
  help                              this text
worker:
  plan <modifies> <uses> -- <done when ...>
                                    register intent ('-' for none; queue order = plan order) and
                                    what done means for your task: a reviewer checks your work
                                    against it before it lands. The first plan, from anywhere in
                                    the project, joins you (creating the board if none) under a
                                    free name agent-<n>, or the name given as
                                    `plan <me> <modifies> <uses> -- ...`, and prints
                                    WORKSPACE <me> <path>: run everything else there
  claim <fn>                        wait for your turn on fn; merges its latest change in
  done <fn> <note...>               record your work as a change, the contract note, pass fn on
  finish                            wait until functions you use settle; exit 10 = new contract
                                    changes (already merged in): adapt, re-test, finish again.
                                    The round's last finish combines everyone's work: exit 12 =
                                    you review it before it lands
  review                            what you review (exit 12 while your approval is missing)
  land <evidence...>                approve your part of the review: evidence per criterion
                                    (file:line); the round lands when every reviewer approved
  reject <agent> <reason...>        nothing lands; the agent's work goes back to it to fix
  join <url>                        on another machine: copy the project for the agent the URL
                                    names (printed as WORKSPACE <agent> <path>)
anyone:
  status                            team, plans, holder/queue per function, finished agents,
                                    the waiting review, minutes since each agent's last command
  log                               event log (REVIEW / APPROVE / REJECT / LANDED / GATE FAIL)
  show <change> [<file>]            a change's files, or one file's content
  diff <from> <to>                  what changed between two changes (unified diff)
orchestrator (optional):
  init [<agent>...]                 set the project up, reset the board, one workspace per agent
  add <agent>                       a new agent joins the running team; prints WORKSPACE, and
                                    JOIN while serving
  release <agent>                   drop a crashed agent from the team and every queue
  serve [port]                      serve board + files to agents on other machines (default
                                    port 7457); prints JOIN <agent> <command> per agent
  hook claude                       Claude Code PreToolUse hook on edits: tells a session editing
                                    the project folder that agents are working and how to join
claim/finish wait at most 100 s per call (exit 11: run the same command again). An agent they wait
on that ran no command for 30 min stops the wait (exit 1, BLOCKED by <agent>): release it if it crashed
exit codes: 0 ok, 10 re-check required, 11 still waiting: run again, 12 review required, 1 protocol
error / blocked / combine FAIL, 2 usage or setup error";

const POLL: Duration = Duration::from_secs(2);
/// A waiting command returns after this long (exit 11, run it again), well inside an agent
/// tool's command timeout (Claude Code's default is 2 min), so no wait is ever killed midway.
const WAIT_LIMIT: Duration = Duration::from_secs(100);

pub(crate) const EXIT_ERROR: u8 = 1;
pub(crate) const EXIT_SETUP: u8 = 2;
const EXIT_RECHECK: u8 = 10;
const EXIT_WAIT: u8 = 11;
const EXIT_REVIEW: u8 = 12;

/// Run the command line of this process.
pub fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => ExitCode::from(code),
        Err(Failure { code, message }) => {
            err_out!("{message}");
            ExitCode::from(code)
        }
    }
}

pub(crate) struct Failure {
    pub(crate) code: u8,
    pub(crate) message: String,
}

fn setup(message: impl Into<String>) -> Failure {
    Failure {
        code: EXIT_SETUP,
        message: message.into(),
    }
}

fn protocol(message: impl Into<String>) -> Failure {
    Failure {
        code: EXIT_ERROR,
        message: message.into(),
    }
}

fn err(e: impl std::fmt::Display) -> Failure {
    setup(format!("ERROR {e}"))
}

fn run(args: &[String]) -> Result<u8, Failure> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let here = Path::new(".");
    match words.as_slice() {
        ["help" | "-h" | "--help"] => {
            out!("{USAGE}");
            Ok(0)
        }
        ["init", team @ ..] => cmd_init(&owned(team)),
        ["serve"] => cmd_serve(serve::DEFAULT_PORT),
        ["serve", port] => cmd_serve(port.parse().map_err(|_| setup(USAGE))?),
        ["join", url] => cmd_join(url),
        ["hook", "claude"] => Ok(cmd_hook_claude()),
        ["log"] => cmd_log(link(here)?.board()?),
        ["status"] => cmd_status(link(here)?.board()?),
        ["show", change] => cmd_show(change, None),
        ["show", change, file] => cmd_show(change, Some(file)),
        ["diff", from, to] => cmd_diff(from, to),
        ["add", agent] => cmd_add(agent),
        ["release", agent] => Ok(report(&link(here)?.apply(&Command::Release {
            agent: agent.to_string(),
        })?)),
        ["plan", modifies, uses, "--", done @ ..] => cmd_plan(None, modifies, uses, done),
        ["plan", agent, modifies, uses, "--", done @ ..] => {
            cmd_plan(Some(agent), modifies, uses, done)
        }
        ["plan", ..] => Err(protocol(core::Error::NoCriteria.to_string())),
        ["claim", func] => cmd_claim(&link(here)?, func),
        ["done", func, note @ ..] if !note.is_empty() => {
            cmd_done(&link(here)?, func, &note.join(" "))
        }
        ["finish"] => cmd_finish(&link(here)?),
        ["review"] => cmd_review(&link(here)?),
        ["land", evidence @ ..] if !evidence.is_empty() => {
            cmd_land(&link(here)?, &evidence.join(" "))
        }
        ["reject", author, reason @ ..] if !reason.is_empty() => {
            cmd_reject(&link(here)?, author, &reason.join(" "))
        }
        _ => Err(setup(USAGE)),
    }
}

/// Where this folder's board lives: in the project's `.concord` (shared by every local
/// workspace), or on the server a `concord join` clone points to.
enum Link {
    Local(Store),
    Remote(Remote),
}

fn link(dir: &Path) -> Result<Link, Failure> {
    let place = store::locate(dir).map_err(err)?;
    Ok(match remote::load(&place.board) {
        Some(remote) => Link::Remote(remote),
        None => Link::Local(Store::new(place.board)),
    })
}

/// The folder this command runs in (the nearest one concord works in).
fn worktree_here() -> Result<Worktree, Failure> {
    let place = store::locate(Path::new(".")).map_err(err)?;
    Worktree::open(&place.folder).map_err(err)
}

impl Link {
    fn apply(&self, cmd: &Command) -> Result<Outcome, Failure> {
        match self {
            Link::Local(store) => execute(store, cmd),
            Link::Remote(remote) => match remote.command(cmd).map_err(setup)? {
                Reply::Ok(outcome) => Ok(outcome),
                Reply::Err { code, message } => Err(Failure { code, message }),
            },
        }
    }

    fn board(&self) -> Result<Option<Board>, Failure> {
        match self {
            Link::Local(store) => store
                .load()
                .map_err(|e| setup(format!("ERROR board I/O: {e}"))),
            Link::Remote(remote) => remote.board().map_err(setup),
        }
    }

    /// The agent this folder belongs to: the agent workspace it is in, or the agent a joined
    /// clone was made for. `None` in the main folder.
    fn agent_here(&self) -> Result<Option<String>, Failure> {
        match self {
            Link::Remote(remote) => Ok(Some(remote.agent.clone())),
            Link::Local(_) => {
                let here = worktree_here()?;
                let state = here.state();
                Ok(state.store.is_some().then(|| state.name.clone()))
            }
        }
    }

    /// Worker commands act for the agent whose workspace they run in.
    fn me(&self) -> Result<String, Failure> {
        self.agent_here()?.ok_or_else(|| {
            protocol(
                "ERROR run this inside your workspace (the WORKSPACE path your first \
                 `concord plan` printed); start with `concord plan <modifies> <uses>`",
            )
        })
    }

    /// Make `change` readable by every agent and the gate: from a joined clone, upload what the
    /// server lacks. Local workspaces share the project's store already.
    fn publish(&self, objects: &Objects, change: &str) -> Result<(), Failure> {
        match self {
            Link::Local(_) => Ok(()),
            Link::Remote(remote) => remote.push(objects, change).map_err(err),
        }
    }

    /// Get other agents' changes before reading or merging them.
    fn sync(&self, objects: &Objects, changes: &[&str]) -> Result<(), Failure> {
        match self {
            Link::Local(_) => Ok(()),
            Link::Remote(remote) => changes
                .iter()
                .try_for_each(|change| remote.fetch(objects, change))
                .map_err(err),
        }
    }
}

/// Plan; an agent's first plan joins it to the project (see `join_if_needed`).
fn cmd_plan(agent: Option<&str>, modifies: &str, uses: &str, done: &[&str]) -> Result<u8, Failure> {
    let (agent, workspace) = join_if_needed(agent)?;
    let link = link(Path::new("."))?;
    let cmd = Command::Plan {
        agent,
        modifies: csv(modifies),
        uses: csv(uses),
        criteria: done.join(" "),
        session: session_here(matches!(link, Link::Local(_))),
    };
    let code = report(&link.apply(&cmd)?);
    if let Some(path) = workspace {
        out!(
            "-> from now on work in {}: claim, edit, done, finish there",
            path.display()
        );
    }
    Ok(code)
}

fn cmd_claim(link: &Link, func: &str) -> Result<u8, Failure> {
    let agent = link.me()?;
    let outcome = link.apply(&Command::Claim {
        agent: agent.clone(),
        func: func.to_string(),
    })?;
    let code = report(&outcome);
    if let Outcome::Claimed {
        latest: Some(c), ..
    } = &outcome
    {
        let mut here = worktree_here()?;
        link.sync(here.objects(), &[&c.change])?;
        let conflicts = here.merge_in(std::slice::from_ref(&c.change)).map_err(err)?;
        out!("MERGED {}", short(&c.change));
        report_conflicts(&conflicts, "before editing");
    }
    Ok(code)
}

/// Commit the work as `<agent>: <fn>: <note>`, publish it, then record the contract. With nothing
/// new since the last change (two functions edited together, a second `done`), the contract
/// records that change: an empty change on top would hide the agent's work from the gate.
fn cmd_done(link: &Link, func: &str, note: &str) -> Result<u8, Failure> {
    let agent = link.me()?;
    let agent = agent.as_str();
    let mut cmd = Command::Done {
        agent: agent.to_string(),
        func: func.to_string(),
        change: String::new(),
        note: note.to_string(),
    };
    // A command the board would reject must not leave a change behind.
    core::apply(link.board()?, &cmd, now()).map_err(|e| protocol(e.to_string()))?;
    let mut here = worktree_here()?;
    refuse_conflicts(&here)?;
    let change = match here.state().head.clone() {
        Some(head) if !here.has_work().map_err(err)? => head,
        _ => here
            .commit(agent, &format!("{agent}: {func}: {note}"))
            .map_err(err)?,
    };
    link.publish(here.objects(), &change)?;
    if let Command::Done { change: c, .. } = &mut cmd {
        *c = change;
    }
    Ok(report(&link.apply(&cmd)?))
}

/// Files with merge conflicts are settled before they go into a change.
fn refuse_conflicts(here: &Worktree) -> Result<(), Failure> {
    let open = here.unresolved().map_err(err)?;
    if open.is_empty() {
        return Ok(());
    }
    report_conflicts(&open, "first");
    Err(protocol(
        "ERROR unresolved conflicts: settle the files listed after CONFLICTS, then run this again",
    ))
}

/// The coding-agent session this command runs in, from the variables its harness sets:
/// Claude Code `CLAUDE_CODE_SESSION_ID`, Codex `CODEX_THREAD_ID` (or `CODEX_SESSION_ID`).
fn session_here(local: bool) -> Option<Session> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let (harness, id) = if let Some(id) = var("CLAUDE_CODE_SESSION_ID") {
        (Harness::Claude, id)
    } else {
        (Harness::Codex, var("CODEX_THREAD_ID").or_else(|| var("CODEX_SESSION_ID"))?)
    };
    Some(Session { harness, id, local })
}

/// Commit leftover work, publish the agent's head, wait for the functions it uses to settle.
/// The last agent of a round to finish combines the round's work and reviews it (exit 12).
fn cmd_finish(link: &Link) -> Result<u8, Failure> {
    let agent = link.me()?;
    let agent = agent.as_str();
    let mut here = worktree_here()?;
    refuse_conflicts(&here)?;
    if here.has_work().map_err(err)? {
        here.commit(agent, &format!("{agent}: finish")).map_err(err)?;
    }
    let head = here
        .state()
        .head
        .clone()
        .ok_or_else(|| setup("ERROR this workspace has no change yet"))?;
    link.publish(here.objects(), &head)?;
    let outcome = link.apply(&Command::Finish {
        agent: agent.to_string(),
        change: head,
    })?;
    let code = report(&outcome);
    if let Outcome::Finished { unseen, .. } = &outcome
        && !unseen.is_empty()
    {
        let changes: Vec<String> = unseen.iter().map(|c| c.change.clone()).collect();
        let refs: Vec<&str> = changes.iter().map(String::as_str).collect();
        link.sync(here.objects(), &refs)?;
        let conflicts = here.merge_in(&changes).map_err(err)?;
        let shown: Vec<&str> = changes.iter().map(|c| short(c)).collect();
        out!("MERGED {}", shown.join(" "));
        report_conflicts(&conflicts, "first");
        out!("-> adapt your code to the notes above, re-run tests, then: concord finish");
    }
    if code != 0 {
        return Ok(code);
    }
    if let Link::Local(store) = link {
        start_review_if_due(store, agent);
    }
    show_review(link, agent)
}

/// Print what `agent` reviews; exit 12 while its approval is missing, else 0.
fn show_review(link: &Link, agent: &str) -> Result<u8, Failure> {
    let board = link.board()?;
    if let Some(reason) = board
        .as_ref()
        .filter(|b| b.team.iter().any(|a| a == agent))
        .and_then(|b| b.combine_failed.as_ref())
    {
        out!("FAIL the round cannot be combined: {reason}");
        out!("-> nothing landed; fix it in your workspace, then: concord finish");
        return Ok(EXIT_ERROR);
    }
    let Some(task) = board.as_ref().and_then(|b| core::assignment(b, agent)) else {
        return Ok(0);
    };
    if task.approved {
        out!("REVIEW approved by {agent}; waiting for the other reviewers (concord status)");
        return Ok(0);
    }
    // A joined clone reads the round through its own store: get what the review names.
    let here = worktree_here()?;
    let mut named: Vec<&str> = vec![&task.combined];
    named.extend(task.base.as_deref());
    named.extend(task.criteria.iter().map(|(_, head, _, _)| head.as_str()));
    link.sync(here.objects(), &named)?;
    let combined = short(&task.combined);
    let base = task.base.as_deref().map_or("<none>", short);
    out!("REVIEW REQUIRED {agent}: check this round's combined work before it lands");
    let checkout = match link {
        Link::Local(store) => store
            .main_root()
            .map(gate::checkout)
            .filter(|p| p.is_dir()),
        Link::Remote(_) => None,
    };
    match checkout {
        Some(path) => out!("COMBINED {combined} checked out at {}", path.display()),
        None => out!("COMBINED {combined}"),
    }
    out!("  read it: concord show {combined} [<file>], concord diff {base} {combined}");
    out!("CHECK each agent's done-criteria against that agent's own work (concord diff {base} <head>):");
    for (who, head, first, current) in &task.criteria {
        out!("  {who} at {}: {current}", short(head));
        if first != current {
            out!("    CRITERIA CHANGED after they were first stated; first: {first}");
        }
    }
    if !task.contracts.is_empty() {
        out!("CHECK the combined result against every contract change of the round (is every caller adapted?):");
        for c in &task.contracts {
            out!("  {} by {}: {}", c.id, c.agent, c.note);
        }
    }
    out!(
        "-> then: concord land \"<evidence per criterion, file:line>\"   (the round lands when every reviewer approved)"
    );
    out!("   or:   concord reject <agent> \"<what is wrong>\"   (nothing lands; the agent fixes it)");
    Ok(EXIT_REVIEW)
}

fn cmd_review(link: &Link) -> Result<u8, Failure> {
    let agent = link.me()?;
    let code = show_review(link, &agent)?;
    if code == 0 && link.board()?.and_then(|b| b.review).is_none() {
        out!("NO REVIEW waiting");
    } else if code == 0 {
        out!("NO REVIEW for {agent}");
    }
    Ok(code)
}

fn cmd_land(link: &Link, evidence: &str) -> Result<u8, Failure> {
    let cmd = Command::Approve {
        agent: link.me()?,
        evidence: evidence.to_string(),
    };
    match link {
        Link::Local(store) => approve(store, &cmd).map(|_| 0),
        Link::Remote(_) => Ok(report(&link.apply(&cmd)?)),
    }
}

/// A reviewer's approval; the one completing the review lands the round. A review already landing
/// (another reviewer's `land` is pulling it, or one was interrupted) is landed instead, after that
/// one: running `land` again finishes an interrupted landing. Prints what it did.
pub(crate) fn approve(store: &Store, cmd: &Command) -> Result<Outcome, Failure> {
    let Command::Approve { agent, .. } = cmd else {
        return Err(setup("ERROR not an approval"));
    };
    let landing = store
        .load()
        .map_err(|e| setup(format!("ERROR board I/O: {e}")))?
        .and_then(|b| b.review)
        .filter(|r| r.landing && r.reviewers.contains_key(agent));
    if let Some(review) = landing {
        out!("LANDING {} was approved; finishing it", short(&review.combined));
        land_round(store, &review.combined)?;
        return Ok(Outcome::Approved {
            agent: agent.clone(),
            complete: true,
            combined: review.combined,
        });
    }
    let outcome = execute(store, cmd)?;
    report(&outcome);
    if let Outcome::Approved {
        complete: true,
        combined,
        ..
    } = &outcome
    {
        land_round(store, combined)?;
    }
    Ok(outcome)
}

fn cmd_reject(link: &Link, author: &str, reason: &str) -> Result<u8, Failure> {
    let agent = link.me()?;
    let outcome = link.apply(&Command::Reject {
        agent,
        author: author.to_string(),
        reason: reason.to_string(),
    })?;
    if let Link::Local(store) = link {
        after_reject(store, &outcome);
    }
    Ok(report(&outcome))
}

/// The round's last finish: combine every finished head (Mergiraf, lost work, conflicts) and
/// start the review. A second reviewer's session is continued to review its part.
pub(crate) fn start_review_if_due(store: &Store, by: &str) {
    // The board is read under the gate lock: of two finishes at once, the second sees the review
    // the first started and combines nothing.
    let Ok(_one_gate_at_a_time) = store.gate_lock() else {
        return;
    };
    let Ok(Some(board)) = store.load() else {
        return;
    };
    if !core::review_due(&board) {
        return;
    }
    let Some(root) = store.main_root() else {
        return;
    };
    out!("ROUND complete: combining everyone's work for review");
    let heads = core::finished_heads(&board);
    let base = board.landing.as_ref().map(|l| l.base.as_str());
    let combined = match gate::combine(root, &heads, base) {
        Ok(gate::Combined::Ready { change }) => change,
        Ok(gate::Combined::Fail { reason }) => {
            let _ = execute(store, &Command::GateFailed { reason });
            return;
        }
        Err(e) => {
            let reason = format!("combining failed: {e}");
            let _ = execute(store, &Command::GateFailed { reason });
            return;
        }
    };
    let started = execute(
        store,
        &Command::StartReview {
            by: by.to_string(),
            combined,
        },
    );
    if let Ok(outcome) = &started {
        report(outcome);
    }
    if let Ok(Outcome::ReviewStarted {
        resume: Some((agent, session)),
        ..
    }) = &started
    {
        let workspace = team::workspace_path(root, agent);
        let prompt = format!(
            "concord: before this round lands, review your part of it. In {}: run `concord \
             review` and follow what it prints (concord land or concord reject).",
            workspace.display()
        );
        resume(store, agent, session, &workspace, &prompt);
    }
}

/// Every reviewer approved `combined`: pull it into the main folder, record the round in the
/// ledger, end the round for its agents. Under the gate lock, on the board as it is then: a
/// landing another process finished meanwhile is not landed twice.
fn land_round(store: &Store, combined: &str) -> Result<(), Failure> {
    let _one_gate_at_a_time = store.gate_lock().map_err(err)?;
    let board = store
        .load()
        .map_err(|e| setup(format!("ERROR board I/O: {e}")))?;
    let landing = |r: &core::Review| r.landing && r.combined == combined;
    let Some((board, review)) = board.and_then(|b| b.review.clone().filter(landing).map(|r| (b, r)))
    else {
        return Ok(());
    };
    let root = store
        .main_root()
        .ok_or_else(|| setup("ERROR cannot find the main folder"))?;
    // The reviewer runs this inside its own workspace, which landing removes: work from the
    // main folder, so nothing after the removal resolves a directory that is gone.
    std::env::set_current_dir(root).map_err(err)?;
    let base = board
        .landing
        .as_ref()
        .map(|l| l.base.clone())
        .ok_or_else(|| setup("ERROR the board records no base"))?;
    gate::land(root, &base, &review.combined).map_err(err)?;
    store
        .append_ledger(&gate::ledger_entry(&board, &review, now()))
        .map_err(err)?;
    for agent in team::remove(root, &review.agents, &review.combined).map_err(err)? {
        out!("REMOVED workspace {agent}");
    }
    report(&execute(
        store,
        &Command::Landed {
            combined: review.combined,
        },
    )?);
    Ok(())
}

/// A rejection voids the combined result; the author's own session gets the reason.
pub(crate) fn after_reject(store: &Store, outcome: &Outcome) {
    let Some(root) = store.main_root() else {
        return;
    };
    if let Err(e) = gate::discard(root) {
        out!("could not drop the review checkout: {e}");
    }
    if let Outcome::Rejected {
        author,
        reason,
        resume: Some(session),
        ..
    } = outcome
    {
        let workspace = team::workspace_path(root, author);
        let prompt = format!(
            "concord: a reviewer rejected your work before it landed: {reason}. Fix it in {} \
             following the concord skill (worker protocol): run `concord plan` again with the \
             functions you will modify and your done-criteria, claim, fix, done, then `concord \
             finish`.",
            workspace.display()
        );
        resume(store, author, session, &workspace, &prompt);
    }
}

/// Continue `agent`'s own coding-agent session with `prompt`, detached (its own process group,
/// so the command that triggered it is not held), output in `resume-<agent>.log` beside the board.
fn resume(store: &Store, agent: &str, session: &Session, workspace: &Path, prompt: &str) {
    use std::os::unix::process::CommandExt as _;
    let words = session.harness.resume_command(&session.id, prompt);
    let log = store.dir().join(format!("resume-{agent}.log"));
    let spawned = (|| -> std::io::Result<()> {
        let (program, args) = words
            .split_first()
            .ok_or_else(|| std::io::Error::other("empty resume command"))?;
        let out = fs::OpenOptions::new().create(true).append(true).open(&log)?;
        std::process::Command::new(program)
            .args(args)
            .current_dir(workspace)
            .stdin(std::process::Stdio::null())
            .stdout(out.try_clone()?)
            .stderr(out)
            .process_group(0)
            .spawn()
            .map(drop)
    })();
    match spawned {
        Ok(()) => out!(
            "RESUMED {agent} in its session {} (output: {})",
            session.id,
            log.display()
        ),
        Err(e) => out!(
            "could not continue {agent}'s session ({e}); run it yourself: {}",
            words.join(" ")
        ),
    }
}

fn report_conflicts(conflicts: &[Conflict], when: &str) {
    if conflicts.is_empty() {
        return;
    }
    let mut paths: Vec<&str> = conflicts.iter().map(|c| c.path.as_str()).collect();
    paths.dedup();
    out!(
        "CONFLICTS {}: settle these {when}: remove the conflict markers; where there is a \
         <file>.theirs, put the right content in <file> and delete <file>.theirs; a file one side \
         deleted and the other changed is kept: keep or delete it, either settles it",
        paths.join(" ")
    );
}

/// Before an agent's first `plan`: no `init` needed. Starts the board here if there is none,
/// else joins the running team, under `agent` or the first free `agent-<n>`; creates the
/// agent's workspace and prints it. Returns the agent and its new workspace (`None` when it was
/// already on the board: its own workspace or joined clone names it).
fn join_if_needed(agent: Option<&str>) -> Result<(String, Option<PathBuf>), Failure> {
    let here = Path::new(".");
    if let Ok(link) = link(here) {
        let named_here = link.agent_here()?;
        let member = |name: &str| match &link {
            Link::Remote(_) => true,
            Link::Local(store) => store
                .load()
                .ok()
                .flatten()
                .is_some_and(|b| b.team.iter().any(|a| a == name)),
        };
        match (agent, named_here) {
            (Some(name), _) if member(name) => return Ok((name.to_string(), None)),
            (None, Some(name)) if member(&name) => return Ok((name, None)),
            (None, Some(name)) => {
                return Err(protocol(format!(
                    "ERROR {name}'s round is over (landed or released): run concord plan from \
                     the project folder to join again"
                )));
            }
            _ => {}
        }
    }
    let root = team::prepare(here).map_err(err)?;
    let store = Store::new(team::store_dir(&root));
    // No board, or a board whose round is over: the base is the main folder as it is now.
    let new_round = store
        .load()
        .map_err(|e| setup(format!("ERROR board I/O: {e}")))?
        .is_none_or(|b| b.team.is_empty());
    let landing = if new_round {
        Some(team::landing(&root).map_err(err)?)
    } else {
        None
    };
    let outcome = execute(
        &store,
        &Command::Join {
            agent: agent.map(str::to_string),
            landing,
        },
    )?;
    let Outcome::Joined { agent, joined } = &outcome else {
        return Err(setup("ERROR joining the team gave no agent"));
    };
    let agent = agent.clone();
    if !joined {
        return Ok((agent, None));
    }
    let base = store
        .load()
        .map_err(|e| setup(format!("ERROR board I/O: {e}")))?
        .and_then(|b| b.landing)
        .map(|l| l.base)
        .ok_or_else(|| setup("ERROR the board records no base"))?;
    let created = team::create(&root, std::slice::from_ref(&agent), &[], &base).map_err(err)?;
    serve::add_token(store.dir(), &agent).map_err(err)?;
    report(&outcome);
    let mut path = None;
    for (agent, p) in created {
        out!("WORKSPACE {agent} {}", p.display());
        path = Some(p);
    }
    Ok((agent, path))
}

/// A new agent joins the running team: a workspace at the team's base, a token for `join`.
fn cmd_add(agent: &str) -> Result<u8, Failure> {
    let root = main_root()?;
    let store = Store::new(team::store_dir(&root));
    let board = store
        .load()
        .map_err(|e| setup(format!("ERROR board I/O: {e}")))?;
    let cmd = Command::Add {
        agent: agent.to_string(),
    };
    // Checked before touching workspaces: re-adding a member would replace its workspace.
    core::apply(board.clone(), &cmd, now()).map_err(|e| protocol(e.to_string()))?;
    let base = board
        .and_then(|b| b.landing)
        .map(|l| l.base)
        .ok_or_else(|| setup("ERROR the board records no base; run concord init again"))?;
    let workspaces = team::create(&root, &[agent.to_string()], &[], &base).map_err(err)?;
    let token = serve::add_token(store.dir(), agent).map_err(err)?;
    let outcome = execute(&store, &cmd)?;
    report(&outcome);
    for (agent, path) in workspaces {
        out!("WORKSPACE {agent} {}", path.display());
    }
    if let Some(url) = serve::live_url(store.dir()) {
        out!("JOIN {agent} concord join {url}/t/{token}");
    }
    Ok(0)
}

fn cmd_init(agents: &[String]) -> Result<u8, Failure> {
    let root = team::prepare(Path::new(".")).map_err(err)?;
    // `init` resets the whole board: from inside an agent's workspace it would end the round of
    // every agent, so it runs only in the project folder.
    if let Ok(place) = store::locate(Path::new("."))
        && place.folder != root
    {
        return Err(protocol(format!(
            "ERROR init resets the project's board; run it in the project folder {}, not in {}",
            root.display(),
            place.folder.display()
        )));
    }
    let landing = team::landing(&root).map_err(err)?;
    let store = Store::new(team::store_dir(&root));
    let previous = store
        .load()
        .map_err(|e| setup(format!("ERROR board I/O: {e}")))?
        .map(|b| b.team)
        .unwrap_or_default();
    let base = landing.base.clone();
    let outcome = execute(
        &store,
        &Command::Init {
            team: agents.to_vec(),
            landing: Some(landing),
        },
    )?;
    let workspaces = team::create(&root, agents, &previous, &base).map_err(err)?;
    serve::issue_tokens(store.dir(), agents).map_err(err)?;
    report(&outcome);
    out!("BASE {} ({})", crate::objects::short(&base), root.display());
    for (agent, path) in workspaces {
        out!("WORKSPACE {agent} {}", path.display());
    }
    Ok(0)
}

/// The main folder of the project this command runs in.
fn main_root() -> Result<PathBuf, Failure> {
    let place = store::locate(Path::new(".")).map_err(err)?;
    place
        .board
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| setup("ERROR cannot find the main folder"))
}

/// Apply `cmd` under the board lock; on `Wait`, poll outside the lock (the waiting agent stays
/// alive on the board) until it can proceed, until an agent it waits on has been silent for
/// `SILENCE_LIMIT` (it may have crashed), or for at most `WAIT_LIMIT` (exit 11: run it again).
pub(crate) fn execute(store: &Store, cmd: &Command) -> Result<Outcome, Failure> {
    let started = std::time::Instant::now();
    loop {
        let now = now();
        let result = store
            .transact(|board| match core::apply(board.clone(), cmd, now) {
                Ok(Step::Next(board, outcome)) => (Some(*board), Ok(Ok(outcome))),
                Ok(Step::Wait) => {
                    let stalled = board
                        .as_ref()
                        .and_then(|b| core::stalled(b, cmd, now, core::SILENCE_LIMIT));
                    (core::heartbeat(board, cmd, now), Ok(Err(stalled)))
                }
                Err(e) => (None, Err(e)),
            })
            .map_err(|e| setup(format!("ERROR board I/O: {e}")))?;
        match result {
            Ok(Ok(outcome)) => return Ok(outcome),
            Ok(Err(None)) if started.elapsed() >= WAIT_LIMIT => {
                return Err(Failure {
                    code: EXIT_WAIT,
                    message: format!(
                        "WAITING for {}: nothing to do meanwhile; run the same command again",
                        waiting_for(cmd)
                    ),
                });
            }
            Ok(Err(None)) => sleep(POLL),
            Ok(Err(Some((agent, silent)))) => {
                return Err(protocol(format!(
                    "BLOCKED by {agent}: no concord command from it for {} min while waiting for {}. \
                     Retry later; if {agent} crashed, the orchestrator runs: concord release {agent}",
                    silent / 60,
                    waiting_for(cmd)
                )));
            }
            Err(e) => return Err(protocol(e.to_string())),
        }
    }
}

fn waiting_for(cmd: &Command) -> String {
    match cmd {
        Command::Claim { func, .. } => format!("turn on {func}"),
        _ => "callers' dependencies to settle".to_string(),
    }
}

/// Print the outcome the way the prototype does; return the exit code.
fn report(outcome: &Outcome) -> u8 {
    match outcome {
        Outcome::Initialized { team } => out!("INIT {}", team.join(" ")),
        Outcome::Planned {
            agent,
            modifies,
            uses,
            finished_users,
        } => {
            out!(
                "PLANNED {agent} modifies={} uses={}",
                py_list(modifies),
                py_list(uses)
            );
            if !finished_users.is_empty() {
                out!(
                    "NOTE already finished using these, checked only by the review: {}",
                    finished_users.join(", ")
                );
            }
        }
        Outcome::Claimed { func, latest } => {
            out!("CLAIMED {func}");
            if let Some(c) = latest {
                out!("LATEST {}  by {}: {}", short(&c.change), c.agent, c.note);
            }
        }
        Outcome::Released {
            func,
            next,
            callers,
        } => out!(
            "RELEASED {func}; next holder: {}; callers that will be told: {}",
            next.as_deref().unwrap_or("none"),
            list_or_none(callers)
        ),
        Outcome::Finished {
            agent,
            change,
            unseen,
        } => {
            if unseen.is_empty() {
                out!(
                    "FINISHED {agent} at {}: no unseen contract changes",
                    short(change)
                );
                return 0;
            }
            out!("NEW CONTRACT CHANGES in functions you call:");
            for c in unseen {
                out!("  {} by {} at {}: {}", c.id, c.agent, short(&c.change), c.note);
            }
            return EXIT_RECHECK;
        }
        Outcome::Added { agent } => out!("ADDED {agent}"),
        Outcome::Joined { agent, joined } => {
            if *joined {
                out!("JOINED {agent}");
            }
        }
        Outcome::Landed { agents, remaining } => {
            out!(
                "ROUND {} landed; still working: {}",
                agents.join(", "),
                list_or_none(remaining)
            );
        }
        Outcome::GateFailed => {}
        Outcome::ReviewStarted { review, .. } => {
            let reviewers: Vec<String> = review.reviewers.keys().cloned().collect();
            out!("REVIEW {} by {}", short(&review.combined), reviewers.join(", "));
            let by = &review.contracts_by;
            if review.reviewers.get(by).is_some_and(|own| own.contains(by)) {
                out!("SELF-REVIEWED {by}: no other agent's own session to hand its criteria to");
            }
        }
        Outcome::Approved {
            agent, complete, ..
        } => {
            out!("APPROVED by {agent}");
            if !complete {
                out!("waiting for the other reviewers (concord status)");
            }
        }
        Outcome::Rejected {
            author,
            reason,
            blocked,
            session,
            resume,
        } => {
            out!("REJECTED {author}: {reason}");
            if *blocked {
                let again = session.as_ref().map_or_else(String::new, |s| {
                    format!(": {}", s.harness.resume_command(&s.id, reason).join(" "))
                });
                out!(
                    "BLOCKED {author}: rejected more than {} times this round; a person decides \
                     how to continue{again}",
                    core::REJECT_LIMIT
                );
                return EXIT_ERROR;
            }
            if resume.is_none() {
                out!("-> {author} fixes it in its workspace, then concord finish");
            }
        }
        Outcome::Removed { agent, next } => {
            out!("RELEASED {agent}: removed from the team");
            for (func, holder) in next {
                out!("  {func} now held by {holder}");
            }
        }
    }
    0
}

fn cmd_log(board: Option<Board>) -> Result<u8, Failure> {
    match board {
        Some(b) => out!("{}", b.log.join("\n")),
        None => out!("(empty)"),
    }
    Ok(0)
}

fn cmd_status(board: Option<Board>) -> Result<u8, Failure> {
    match board {
        Some(b) => out!("{}", core::status_lines(&b, now()).join("\n")),
        None => out!("(empty)"),
    }
    Ok(0)
}

fn cmd_serve(port: u16) -> Result<u8, Failure> {
    let root = main_root()?;
    serve::run(&root, &team::store_dir(&root), port).map_err(err)?;
    Ok(0)
}

/// Copy the project behind `url` for the agent it names: a folder of its own with its own store,
/// holding the agents' base change. An existing copy of this agent is reused: one already joined
/// with this token stays as it is; otherwise (a join that failed midway, or the agent was added
/// again with a new token) it is brought to the base, its earlier work kept first (`team::keep`).
fn cmd_join(url: &str) -> Result<u8, Failure> {
    let who = remote::whoami(url).map_err(setup)?;
    let cwd = std::env::current_dir().map_err(err)?;
    let dir = cwd.join(format!("{}-{}", who.project, who.agent));
    let mut copy = if dir.exists() {
        Worktree::open(&dir)
            .ok()
            .filter(|c| c.state().name == who.agent && c.state().store.is_none())
            .ok_or_else(|| {
                setup(format!(
                    "ERROR {} exists and is not a concord copy of {}; move it away",
                    dir.display(),
                    who.agent
                ))
            })?
    } else {
        Worktree::create(&dir, &who.agent, None).map_err(err)?
    };
    let board_dir = dir.join(".concord");
    let joined = copy.state().head.is_some()
        && remote::load(&board_dir).is_some_and(|r| remote::token(&r.url) == remote::token(url));
    if !joined {
        let remote = Remote {
            url: url.to_string(),
            agent: who.agent.clone(),
        };
        remote.fetch(copy.objects(), &who.base).map_err(err)?;
        team::keep(&mut copy, &who.agent, &who.base).map_err(err)?;
        copy.materialize(&who.base).map_err(err)?;
        remote::save(&board_dir, &remote).map_err(err)?;
    }
    out!("WORKSPACE {} {}", who.agent, dir.display());
    Ok(0)
}

/// `concord show <change> [<file>]`: the change's paths, or one file's bytes.
fn cmd_show(change: &str, file: Option<&str>) -> Result<u8, Failure> {
    let here = worktree_here()?;
    let objects = here.objects();
    let tree = objects
        .resolve(change)
        .and_then(|id| objects.change_tree(&id))
        .map_err(err)?;
    let Some(file) = file else {
        for path in tree.keys() {
            out!("{path}");
        }
        return Ok(0);
    };
    let entry = tree
        .get(file)
        .ok_or_else(|| protocol(format!("ERROR {file} is not in {change}")))?;
    let bytes = objects.get(&entry.blob).map_err(err)?;
    // A reader that went away (`| head`) is not an error, same as `out!`.
    let _ = std::io::stdout().write_all(&bytes);
    Ok(0)
}

/// `concord diff <from> <to>`: unified diff of every path that differs.
fn cmd_diff(from: &str, to: &str) -> Result<u8, Failure> {
    let here = worktree_here()?;
    let objects = here.objects();
    let tree = |change: &str| {
        objects
            .resolve(change)
            .and_then(|id| objects.change_tree(&id))
            .map_err(err)
    };
    let text = diff::unified(objects, &tree(from)?, &tree(to)?).map_err(err)?;
    // A reader that went away (`| head`) is not an error, same as `out!`.
    let _ = std::io::stdout().write_all(text.as_bytes());
    Ok(0)
}

/// Claude Code PreToolUse hook on file edits (JSON on stdin). An edit in a project's main folder
/// while agents work on its board gets one note per session: how to coordinate. Never blocks,
/// never fails the edit: anything unexpected means no note.
fn cmd_hook_claude() -> u8 {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return 0;
    }
    if let Some(note) = hook_note(&input) {
        let out = serde_json::json!({
            "hookSpecificOutput": {"hookEventName": "PreToolUse", "additionalContext": note}
        });
        // A closed stdout must not turn into a failed edit.
        let _ = writeln!(std::io::stdout(), "{out}");
    }
    0
}

fn hook_note(input: &str) -> Option<String> {
    let event: serde_json::Value = serde_json::from_str(input).ok()?;
    let tool = &event["tool_input"];
    let file = tool["file_path"].as_str().or(tool["notebook_path"].as_str())?;
    let session = event["session_id"].as_str().unwrap_or("unknown");
    // The file may not exist yet (Write): start from its nearest existing folder.
    let dir = Path::new(file).ancestors().skip(1).find(|d| d.is_dir())?;
    let board_dir = store::locate(dir).ok()?.board;
    let root = fs::canonicalize(board_dir.parent()?).ok()?;
    if !fs::canonicalize(dir).ok()?.starts_with(&root) {
        return None; // an agent workspace or a joined clone: already coordinated
    }
    let board = Store::new(board_dir.clone()).load().ok()??;
    if board.team.is_empty() {
        return None;
    }
    let seen = board_dir.join("hook-sessions");
    let told = fs::read_to_string(&seen).unwrap_or_default();
    if told.lines().any(|l| l == session) {
        return None;
    }
    let mut log = fs::OpenOptions::new().create(true).append(true).open(&seen).ok()?;
    writeln!(log, "{session}").ok()?;
    Some(format!(
        "concord: {} coding agent(s) ({}) are changing this project right now, each in its own \
         workspace; their work lands in this folder when their round passes its tests. If your \
         task changes code they may also touch, coordinate instead of editing here: run \
         `concord plan <functions you modify> <functions you use>` (comma lists, - for none) from \
         this folder, then work in the WORKSPACE it prints, following the concord skill. Edits \
         unrelated to their work can stay here.",
        board.team.len(),
        board.team.join(", ")
    ))
}

fn owned(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

/// Comma list; `-` means none.
fn csv(word: &str) -> Vec<String> {
    if word == "-" {
        Vec::new()
    } else {
        word.split(',')
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect()
    }
}

/// Unix seconds now.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
