//! Pure board state machine: every protocol rule lives here, no I/O.
//! Reference behavior: `prototype/coord.py`; protocol: `docs/design.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::objects::short;

/// Board state, stored as one JSON file. Field names match the prototype's `state.json`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Board {
    pub team: Vec<String>,
    pub plans: BTreeMap<String, Plan>,
    /// Per modified function: agents in plan order; the first one holds the function.
    pub queue: BTreeMap<String, Vec<String>>,
    /// Per function: contract changes recorded by `done`, oldest first.
    pub history: BTreeMap<String, Vec<Contract>>,
    /// Per agent: contract ids the agent has been shown.
    pub seen: BTreeMap<String, Vec<String>>,
    /// Per agent: the change it finished at; the gate combines these.
    #[serde(default)]
    pub heads: BTreeMap<String, String>,
    /// Where the agents started and where a passing gate lands their work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landing: Option<Landing>,
    /// Unix seconds of `init`, and of each agent's last command that changed the board: who has
    /// gone silent (see `stalled`).
    #[serde(default)]
    pub started: u64,
    #[serde(default)]
    pub active: BTreeMap<String, u64>,
    /// Agents whose last `finish` reported contract changes (exit 10): their head is not final
    /// until they finish again.
    #[serde(default)]
    pub recheck: BTreeSet<String>,
    /// The round's combined result waiting for its reviewers (`review_due` starts one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<Review>,
    /// Per agent: how often a reviewer rejected its work this round (`REJECT_LIMIT`).
    #[serde(default)]
    pub rejections: BTreeMap<String, u32>,
    /// Why combining the current heads failed (lost work, unresolved conflicts). Shown to the
    /// team on `finish`; cleared once a head changes or leaves, so the next combine is due.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combine_failed: Option<String>,
    pub log: Vec<String>,
}

/// A combined result waiting for approval. `reviewers`: per reviewer, the agents whose
/// done-criteria it checks, each against that agent's own head. `contracts_by` also checks the
/// combined result against every contract change of the round.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Review {
    pub combined: String,
    /// Agents whose heads the combined result holds: they land together.
    pub agents: Vec<String>,
    pub reviewers: BTreeMap<String, Vec<String>>,
    pub contracts_by: String,
    /// Per reviewer that approved: its evidence.
    pub approved: BTreeMap<String, String>,
    /// Every reviewer approved and the main folder is being pulled: the review's agents wait
    /// (`finish`, `release`, `land`, `reject`) until `Landed` ends the round.
    #[serde(default, skip_serializing_if = "is_false")]
    pub landing: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The coding-agent session an agent runs in, read from its environment at `plan`. `local`: on
/// the machine holding the board, so concord can continue it there.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub harness: Harness,
    pub id: String,
    pub local: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    Claude,
    Codex,
}

impl Harness {
    /// The command that continues session `id` with `prompt`, without a terminal.
    pub fn resume_command(self, id: &str, prompt: &str) -> Vec<String> {
        let words: &[&str] = match self {
            Harness::Claude => &["claude", "--resume", id, "-p", prompt],
            Harness::Codex => &["codex", "exec", "resume", id, prompt],
        };
        words.iter().map(|w| w.to_string()).collect()
    }
}

/// A reviewer's rejections per agent and round that concord acts on by continuing the agent's
/// session; the next one stops with BLOCKED for a person to look.
pub const REJECT_LIMIT: u32 = 2;

/// The change every agent workspace starts from; an approved round is pulled into the main
/// folder on top of it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Landing {
    pub base: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub modifies: Vec<String>,
    pub uses: Vec<String>,
    /// What "done" means for the agent's task, every version it stated, oldest first.
    #[serde(default)]
    pub criteria: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<Session>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Contract {
    pub id: String,
    pub agent: String,
    pub change: String,
    pub note: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Init {
        team: Vec<String>,
        landing: Option<Landing>,
    },
    Plan {
        agent: String,
        modifies: Vec<String>,
        uses: Vec<String>,
        criteria: String,
        session: Option<Session>,
    },
    Claim {
        agent: String,
        func: String,
    },
    Done {
        agent: String,
        func: String,
        change: String,
        note: String,
    },
    Finish {
        agent: String,
        change: String,
    },
    /// Orchestrator: drop a crashed agent from the team, its plan and every queue.
    Release {
        agent: String,
    },
    /// Orchestrator: a new agent joins the running team (or replaces a released one).
    Add {
        agent: String,
    },
    /// An agent joins on its own (its first `plan`): starts the board at `landing` if there is
    /// none, else joins the running team like `Add`. A member joining again changes nothing.
    /// Without a name it gets the first free `agent-<n>`.
    Join {
        agent: Option<String>,
        landing: Option<Landing>,
    },
    /// The round's work is combined at `combined` (`review_due`): start its review. `by` is the
    /// agent whose finish completed the round, the one reviewer sure to be running.
    StartReview {
        by: String,
        combined: String,
    },
    /// A reviewer approves its part of the review with its evidence (`concord land`).
    Approve {
        agent: String,
        evidence: String,
    },
    /// A reviewer rejects `author`'s work (`concord reject`): nothing lands, the author fixes it.
    Reject {
        agent: String,
        author: String,
        reason: String,
    },
    /// The landing review's `combined` change was pulled into the main folder and is the new
    /// base: the review's agents leave the board; agents still working stay, and agents joining
    /// later start from it. With nobody left, a fresh round starts. Refused unless a review of
    /// `combined` is landing.
    Landed {
        combined: String,
    },
    /// Combining failed (lost work, unresolved conflicts): recorded in the log, so whoever looks
    /// at the board sees why nothing landed.
    GateFailed {
        reason: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Initialized {
        team: Vec<String>,
    },
    /// `finished_users`: agents that already finished while using a function this plan
    /// modifies; only the gate's tests check them against this agent's change.
    Planned {
        agent: String,
        modifies: Vec<String>,
        uses: Vec<String>,
        finished_users: Vec<String>,
    },
    /// `latest` is the newest contract change on the function: merge it in before editing.
    Claimed {
        func: String,
        latest: Option<Contract>,
    },
    Released {
        func: String,
        next: Option<String>,
        callers: Vec<String>,
    },
    /// Non-empty `unseen` means: merge, adapt, re-test, finish again (exit 10).
    Finished {
        agent: String,
        change: String,
        unseen: Vec<Contract>,
    },
    /// `next`: per function the agent held, the agent that holds it now.
    Removed {
        agent: String,
        next: Vec<(String, String)>,
    },
    Added {
        agent: String,
    },
    /// `joined`: false when the agent was already on the board.
    Joined {
        agent: String,
        joined: bool,
    },
    /// `resume`: a second reviewer whose session concord continues to review its part.
    ReviewStarted {
        review: Review,
        resume: Option<(String, Session)>,
    },
    /// `complete`: every reviewer approved; `combined` lands now.
    Approved {
        agent: String,
        complete: bool,
        combined: String,
    },
    /// `resume`: the author's session, continued with the reason. `blocked`: rejected more than
    /// `REJECT_LIMIT` times this round; a person looks before anything continues.
    Rejected {
        author: String,
        reason: String,
        resume: Option<Session>,
        blocked: bool,
        session: Option<Session>,
    },
    Landed {
        agents: Vec<String>,
        remaining: Vec<String>,
    },
    GateFailed,
}

/// Result of applying a command. `Wait` leaves the board untouched; the caller retries later.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Wait,
    Next(Box<Board>, Outcome),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    NoBoard,
    NotInTeam { agent: String, team: Vec<String> },
    AlreadyInTeam { agent: String },
    BadName { agent: String },
    NotPlannedFunc { agent: String, func: String },
    NotHolder { agent: String, func: String },
    NoPlan { agent: String },
    NoCriteria,
    NoReview,
    NotReviewer { agent: String },
    NotFinished { agent: String },
    NotLanding { combined: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoBoard => write!(f, "ERROR no board; run init first"),
            Error::NotInTeam { agent, team } => {
                write!(f, "ERROR {agent} is not in the team {}", py_list(team))
            }
            Error::AlreadyInTeam { agent } => write!(f, "ERROR {agent} is already in the team"),
            Error::BadName { agent } => write!(
                f,
                "ERROR {agent:?} cannot be an agent name: use letters, digits, '-', '_' or '.' \
                 (not default or concord-gate)"
            ),
            Error::NotPlannedFunc { agent, func } => {
                write!(
                    f,
                    "ERROR {agent} did not plan to modify {func}; run plan first"
                )
            }
            Error::NotHolder { agent, func } => write!(f, "ERROR {agent} does not hold {func}"),
            Error::NoPlan { agent } => write!(f, "ERROR {agent} has not planned; run plan first"),
            Error::NoCriteria => write!(
                f,
                "ERROR plan needs your done-criteria: concord plan <modifies> <uses> -- <done when \
                 ...> (what a reviewer checks against your work before it lands)"
            ),
            Error::NoReview => write!(f, "ERROR no review is waiting"),
            Error::NotReviewer { agent } => {
                write!(f, "ERROR {agent} is not a reviewer of the waiting review")
            }
            Error::NotFinished { agent } => {
                write!(f, "ERROR {agent} has no finished work to reject")
            }
            Error::NotLanding { combined } => {
                write!(f, "ERROR no review of {} is landing", short(combined))
            }
        }
    }
}

/// Apply one command at `now` (Unix seconds; log lines show it as UTC `HH:MM:SS`).
pub fn apply(board: Option<Board>, cmd: &Command, now: u64) -> Result<Step, Error> {
    if let Command::Init { team, landing } = cmd {
        return init(team, landing.clone(), now);
    }
    if let Command::Join { agent, landing } = cmd {
        return join(board, agent, landing.clone(), now);
    }
    let board = board.ok_or(Error::NoBoard)?;
    if waits_for_landing(&board, cmd) {
        return Ok(Step::Wait);
    }
    let step = match cmd {
        Command::Init { .. } => init(&[], None, now),
        Command::Plan {
            agent,
            modifies,
            uses,
            criteria,
            session,
        } => plan(board, agent, modifies, uses, criteria, session, now),
        Command::Claim { agent, func } => claim(board, agent, func, now),
        Command::Done {
            agent,
            func,
            change,
            note,
        } => done(board, agent, func, change, note, now),
        Command::Finish { agent, change } => finish(board, agent, change, now),
        Command::Release { agent } => release(board, agent, now),
        Command::Add { agent } => add(board, agent, now),
        Command::Join { agent, landing } => join(Some(board), agent, landing.clone(), now),
        Command::StartReview { by, combined } => start_review(board, by, combined, now),
        Command::Approve { agent, evidence } => approve(board, agent, evidence, now),
        Command::Reject {
            agent,
            author,
            reason,
        } => reject(board, agent, author, reason, now),
        Command::Landed { combined } => landed(board, combined, now),
        Command::GateFailed { reason } => {
            let mut board = board;
            log(&mut board, now, format!("GATE FAIL {reason}"));
            board.combine_failed = Some(reason.clone());
            Ok(next(board, Outcome::GateFailed))
        }
    }?;
    Ok(match (step, worker(cmd)) {
        (Step::Next(mut board, outcome), Some(agent)) => {
            board.active.insert(agent.to_string(), now);
            Step::Next(board, outcome)
        }
        (step, _) => step,
    })
}

/// The agent a worker command acts for.
fn worker(cmd: &Command) -> Option<&str> {
    match cmd {
        Command::Plan { agent, .. }
        | Command::Claim { agent, .. }
        | Command::Done { agent, .. }
        | Command::Finish { agent, .. }
        | Command::Approve { agent, .. }
        | Command::Reject { agent, .. } => Some(agent),
        Command::Join { agent, .. } => agent.as_deref(),
        Command::Init { .. }
        | Command::Release { .. }
        | Command::Add { .. }
        | Command::StartReview { .. }
        | Command::Landed { .. }
        | Command::GateFailed { .. } => None,
    }
}

/// While the main folder is pulled, nothing may change what the landing review holds: its agents'
/// heads and membership, and the review itself.
fn waits_for_landing(board: &Board, cmd: &Command) -> bool {
    let Some(review) = board.review.as_ref().filter(|r| r.landing) else {
        return false;
    };
    match cmd {
        Command::Finish { agent, .. }
        | Command::Release { agent }
        | Command::Approve { agent, .. }
        | Command::Reject { agent, .. } => review.agents.contains(agent),
        _ => false,
    }
}

/// Every agent has finished with nothing left to re-check and no review is waiting: combine the
/// round and start its review.
pub fn review_due(board: &Board) -> bool {
    board.review.is_none()
        && board.combine_failed.is_none()
        && !board.team.is_empty()
        && board
            .team
            .iter()
            .all(|a| board.heads.contains_key(a) && !board.recheck.contains(a))
}

/// A worker command that has to wait still shows its agent is alive: the board with the
/// agent's clock moved to `now`, or `None` when there is nothing to record (not a member, or
/// recorded less than a minute ago: a poll every 2 s need not rewrite the board each time).
pub fn heartbeat(board: Option<Board>, cmd: &Command, now: u64) -> Option<Board> {
    let agent = worker(cmd)?;
    let mut board = board?;
    let recent = board.active.get(agent).is_some_and(|t| now < t + 60);
    if recent || !board.team.iter().any(|a| a == agent) {
        return None;
    }
    board.active.insert(agent.to_string(), now);
    Some(board)
}

/// Agent names become workspace folders (`<project>-<agent>`) and workspace names.
fn require_valid_name(agent: &str) -> Result<(), Error> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    let reserved = ["default", "concord-gate"].contains(&agent) || agent.starts_with('.');
    if agent.is_empty() || reserved || !agent.chars().all(allowed) {
        return Err(Error::BadName {
            agent: agent.to_string(),
        });
    }
    Ok(())
}

/// The first `agent-<n>` not on the team.
fn free_name(team: &[String]) -> String {
    (1..)
        .map(|n| format!("agent-{n}"))
        .find(|name| !team.contains(name))
        .unwrap_or_default()
}

/// How long an agent that blocks others may stay silent before waiting stops (`stalled`).
pub const SILENCE_LIMIT: u64 = 30 * 60;

/// For a command that has to wait: the first agent it waits on that has been silent for longer
/// than `limit` seconds, with that silence. Waiting on a busy agent is fine; waiting on one that
/// may have crashed is not, since it never runs `done`.
pub fn stalled(board: &Board, cmd: &Command, now: u64, limit: u64) -> Option<(String, u64)> {
    blockers(board, cmd).into_iter().find_map(|agent| {
        let last = board.active.get(&agent).copied().unwrap_or(board.started);
        let silent = now.saturating_sub(last);
        (silent > limit).then_some((agent, silent))
    })
}

/// Agents a waiting command waits on: those who have not planned yet (the barrier), else the
/// holder of the claimed function, or everyone else still holding or queued on a used function.
fn blockers(board: &Board, cmd: &Command) -> Vec<String> {
    let unplanned: Vec<String> = board
        .team
        .iter()
        .filter(|a| !board.plans.contains_key(*a))
        .cloned()
        .collect();
    if !unplanned.is_empty() {
        return unplanned;
    }
    let others_on = |func: &String, me: &str| -> Vec<String> {
        board
            .queue
            .get(func)
            .map(|q| q.iter().filter(|a| a.as_str() != me).cloned().collect())
            .unwrap_or_default()
    };
    match cmd {
        Command::Claim { agent, func } => others_on(func, agent).into_iter().take(1).collect(),
        Command::Finish { agent, .. } => board
            .plans
            .get(agent)
            .map(|p| p.uses.iter().flat_map(|f| others_on(f, agent)).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn release(mut board: Board, agent: &str, now: u64) -> Result<Step, Error> {
    require_member(&board, agent)?;
    board.team.retain(|a| a != agent);
    board.plans.remove(agent);
    board.seen.remove(agent);
    board.heads.remove(agent);
    board.active.remove(agent);
    board.recheck.remove(agent);
    board.rejections.remove(agent);
    work_changed(&mut board, agent, now, "released");
    let mut next_holders = Vec::new();
    for (func, queue) in &mut board.queue {
        let held = queue.first().is_some_and(|a| a == agent);
        queue.retain(|a| a != agent);
        if held && let Some(holder) = queue.first() {
            next_holders.push((func.clone(), holder.clone()));
        }
    }
    board.queue.retain(|_, q| !q.is_empty());
    log(
        &mut board,
        now,
        format!("RELEASE {agent} (removed from the team)"),
    );
    Ok(next(
        board,
        Outcome::Removed {
            agent: agent.to_string(),
            next: next_holders,
        },
    ))
}

/// The new agent's silence clock starts now. Until it plans, `claim` and `finish` wait for it,
/// as at the start (the barrier): nobody finishes before every intent is known.
fn add(mut board: Board, agent: &str, now: u64) -> Result<Step, Error> {
    require_valid_name(agent)?;
    if board.team.iter().any(|a| a == agent) {
        return Err(Error::AlreadyInTeam {
            agent: agent.to_string(),
        });
    }
    board.team.push(agent.to_string());
    board.active.insert(agent.to_string(), now);
    log(&mut board, now, format!("ADD {agent} (joins the team)"));
    Ok(next(
        board,
        Outcome::Added {
            agent: agent.to_string(),
        },
    ))
}

fn join(
    board: Option<Board>,
    agent: &Option<String>,
    landing: Option<Landing>,
    now: u64,
) -> Result<Step, Error> {
    let team = board.as_ref().map(|b| b.team.as_slice()).unwrap_or_default();
    let agent = &agent.clone().unwrap_or_else(|| free_name(team));
    let joined = |board: Board, joined: bool| {
        Ok(next(
            board,
            Outcome::Joined {
                agent: agent.to_string(),
                joined,
            },
        ))
    };
    let Some(board) = board else {
        let team = [agent.to_string()];
        return match init(&team, landing, now)? {
            Step::Next(board, _) => joined(*board, true),
            Step::Wait => Ok(Step::Wait),
        };
    };
    if board.team.iter().any(|a| a == agent) {
        return joined(board, false);
    }
    // The first agent of a new round starts from the main folder as it is now (`landing`), so
    // edits made there since the last round are in the base.
    let mut board = board;
    if board.team.is_empty() && landing.is_some() {
        board.landing = landing;
    }
    match add(board, agent, now)? {
        Step::Next(board, _) => joined(*board, true),
        Step::Wait => Ok(Step::Wait),
    }
}

fn landed(mut board: Board, combined: &str, now: u64) -> Result<Step, Error> {
    let agents = match &board.review {
        Some(review) if review.landing && review.combined == combined => review.agents.clone(),
        _ => {
            return Err(Error::NotLanding {
                combined: combined.to_string(),
            });
        }
    };
    let agents = agents.as_slice();
    let base = combined;
    board.team.retain(|a| !agents.contains(a));
    for agent in agents {
        board.plans.remove(agent);
        board.seen.remove(agent);
        board.heads.remove(agent);
        board.active.remove(agent);
        board.recheck.remove(agent);
        board.rejections.remove(agent);
    }
    board.review = None;
    board.combine_failed = None;
    for queue in board.queue.values_mut() {
        queue.retain(|a| !agents.contains(a));
    }
    board.queue.retain(|_, q| !q.is_empty());
    if let Some(landing) = &mut board.landing {
        landing.base = base.to_string();
    }
    let line = format!("LANDED {} at {}", agents.join(","), short(base));
    if board.team.is_empty() {
        board = Board {
            landing: board.landing,
            started: now,
            ..Board::default()
        };
    }
    log(&mut board, now, line);
    let remaining = board.team.clone();
    Ok(next(
        board,
        Outcome::Landed {
            agents: agents.to_vec(),
            remaining,
        },
    ))
}

fn init(team: &[String], landing: Option<Landing>, now: u64) -> Result<Step, Error> {
    for agent in team {
        require_valid_name(agent)?;
    }
    let board = Board {
        team: team.to_vec(),
        landing,
        started: now,
        log: vec![format!("{} INIT team={}", hms(now), team.join(","))],
        ..Board::default()
    };
    Ok(next(
        board,
        Outcome::Initialized {
            team: team.to_vec(),
        },
    ))
}

fn plan(
    mut board: Board,
    agent: &str,
    modifies: &[String],
    uses: &[String],
    criteria: &str,
    session: &Option<Session>,
    now: u64,
) -> Result<Step, Error> {
    require_member(&board, agent)?;
    let criteria = criteria.trim();
    if criteria.is_empty() {
        return Err(Error::NoCriteria);
    }
    // Re-planning replaces the earlier plan: drop queue slots for functions no longer modified,
    // add a slot only where the agent has none yet (keeps its place in line).
    let old = board
        .plans
        .get(agent)
        .map(|p| p.modifies.clone())
        .unwrap_or_default();
    for func in old.iter().filter(|f| !modifies.contains(f)) {
        if let Some(q) = board.queue.get_mut(func) {
            q.retain(|a| a != agent);
        }
    }
    // Every stated version of the criteria stays: a reviewer sees whether they moved.
    let mut versions = board
        .plans
        .get(agent)
        .map(|p| p.criteria.clone())
        .unwrap_or_default();
    if versions.last().map(String::as_str) != Some(criteria) {
        versions.push(criteria.to_string());
    }
    board.plans.insert(
        agent.to_string(),
        Plan {
            modifies: modifies.to_vec(),
            uses: uses.to_vec(),
            criteria: versions,
            session: session.clone(),
        },
    );
    for func in modifies {
        let q = board.queue.entry(func.clone()).or_default();
        if !q.iter().any(|a| a == agent) {
            q.push(agent.to_string());
        }
    }
    board.seen.entry(agent.to_string()).or_default();
    // Possible only for an agent added after others finished.
    let finished_users: Vec<String> = board
        .heads
        .keys()
        .filter(|a| a.as_str() != agent)
        .filter(|a| {
            board
                .plans
                .get(*a)
                .is_some_and(|p| p.uses.iter().any(|u| modifies.contains(u)))
        })
        .cloned()
        .collect();
    let late = if finished_users.is_empty() {
        String::new()
    } else {
        format!(
            " (already finished using these, checked only by the review: {})",
            finished_users.join(", ")
        )
    };
    log(
        &mut board,
        now,
        format!(
            "PLAN {agent} modifies={} uses={}{late}",
            py_list(modifies),
            py_list(uses)
        ),
    );
    let outcome = Outcome::Planned {
        agent: agent.to_string(),
        modifies: modifies.to_vec(),
        uses: uses.to_vec(),
        finished_users,
    };
    Ok(next(board, outcome))
}

fn claim(mut board: Board, agent: &str, func: &str, now: u64) -> Result<Step, Error> {
    require_member(&board, agent)?;
    if !all_planned(&board) {
        return Ok(Step::Wait);
    }
    let queue = board.queue.get(func).map(Vec::as_slice).unwrap_or_default();
    if !queue.iter().any(|a| a == agent) {
        return Err(Error::NotPlannedFunc {
            agent: agent.to_string(),
            func: func.to_string(),
        });
    }
    if queue.first().map(String::as_str) != Some(agent) {
        return Ok(Step::Wait);
    }
    log(&mut board, now, format!("CLAIM {agent} {func}"));
    let ids: Vec<String> = history(&board, func).iter().map(|c| c.id.clone()).collect();
    mark_seen(&mut board, agent, ids);
    let latest = history(&board, func).last().cloned();
    Ok(next(
        board,
        Outcome::Claimed {
            func: func.to_string(),
            latest,
        },
    ))
}

fn done(
    mut board: Board,
    agent: &str,
    func: &str,
    change: &str,
    note: &str,
    now: u64,
) -> Result<Step, Error> {
    require_member(&board, agent)?;
    let not_holder = || Error::NotHolder {
        agent: agent.to_string(),
        func: func.to_string(),
    };
    let queue = board.queue.get_mut(func).ok_or_else(not_holder)?;
    if queue.first().map(String::as_str) != Some(agent) {
        return Err(not_holder());
    }
    queue.remove(0);
    let next_holder = queue.first().cloned();
    let entries = board.history.entry(func.to_string()).or_default();
    let contract = Contract {
        id: format!("{func}#{}", entries.len() + 1),
        agent: agent.to_string(),
        change: change.to_string(),
        note: note.to_string(),
    };
    entries.push(contract.clone());
    mark_seen(&mut board, agent, vec![contract.id]);
    let callers: Vec<String> = board
        .plans
        .iter()
        .filter(|(a, p)| a.as_str() != agent && p.uses.iter().any(|u| u == func))
        .map(|(a, _)| a.clone())
        .collect();
    log(
        &mut board,
        now,
        format!(
            "DONE {agent} {func} {} :: {note} (callers to re-check: {})",
            short(change),
            list_or_none(&callers)
        ),
    );
    let outcome = Outcome::Released {
        func: func.to_string(),
        next: next_holder,
        callers,
    };
    Ok(next(board, outcome))
}

fn finish(mut board: Board, agent: &str, change: &str, now: u64) -> Result<Step, Error> {
    require_member(&board, agent)?;
    if !all_planned(&board) {
        return Ok(Step::Wait);
    }
    let uses = board
        .plans
        .get(agent)
        .map(|p| p.uses.clone())
        .ok_or_else(|| Error::NoPlan {
            agent: agent.to_string(),
        })?;
    let busy = uses.iter().any(|func| {
        board
            .queue
            .get(func)
            .is_some_and(|q| q.iter().any(|a| a != agent))
    });
    if busy {
        return Ok(Step::Wait);
    }
    let seen = board.seen.get(agent).cloned().unwrap_or_default();
    let unseen: Vec<Contract> = uses
        .iter()
        .flat_map(|func| history(&board, func))
        .filter(|c| !seen.contains(&c.id))
        .cloned()
        .collect();
    mark_seen(
        &mut board,
        agent,
        unseen.iter().map(|c| c.id.clone()).collect(),
    );
    if board.heads.get(agent).map(String::as_str) != Some(change) {
        work_changed(&mut board, agent, now, "finished again");
    }
    board.heads.insert(agent.to_string(), change.to_string());
    if unseen.is_empty() {
        board.recheck.remove(agent);
    } else {
        board.recheck.insert(agent.to_string());
    }
    let ids: Vec<String> = unseen.iter().map(|c| c.id.clone()).collect();
    log(
        &mut board,
        now,
        format!("FINISH {agent} {} new_contracts={}", short(change), py_list(&ids)),
    );
    let outcome = Outcome::Finished {
        agent: agent.to_string(),
        change: change.to_string(),
        unseen,
    };
    Ok(next(board, outcome))
}

/// `agent`'s work changed or left: a waiting review that holds it and a failed combine are void.
fn work_changed(board: &mut Board, agent: &str, now: u64, why: &str) {
    board.combine_failed = None;
    if board
        .review
        .as_ref()
        .is_some_and(|r| r.agents.iter().any(|a| a == agent))
    {
        board.review = None;
        log(board, now, format!("REVIEW CANCELLED {agent} {why}"));
    }
}

/// Assign the review of the combined round. `by` checks every other agent's criteria and the
/// contract changes. Its own criteria go to the other agent whose session concord can continue
/// here, most recently active first; with none, `by` checks its own (logged `SELF-REVIEWED`).
fn start_review(mut board: Board, by: &str, combined: &str, now: u64) -> Result<Step, Error> {
    require_member(&board, by)?;
    if !review_due(&board) {
        return Ok(Step::Wait);
    }
    let agents = board.team.clone();
    let second = agents
        .iter()
        .filter(|a| a.as_str() != by)
        .filter_map(|a| own_session(&board, a).map(|s| (a.clone(), s)))
        .max_by_key(|(a, _)| board.active.get(a).copied().unwrap_or(0));
    let mut reviewers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    reviewers.insert(
        by.to_string(),
        agents.iter().filter(|a| a.as_str() != by).cloned().collect(),
    );
    match &second {
        Some((other, _)) => {
            reviewers.insert(other.clone(), vec![by.to_string()]);
        }
        None => reviewers.entry(by.to_string()).or_default().push(by.to_string()),
    }
    let review = Review {
        combined: combined.to_string(),
        agents,
        reviewers,
        contracts_by: by.to_string(),
        approved: BTreeMap::new(),
        landing: false,
    };
    let line = match &second {
        Some((other, s)) => {
            format!(
                "REVIEW {} by {by}; {by}'s criteria by {other} (RESUME {other} {})",
                short(combined),
                s.id
            )
        }
        None => format!(
            "REVIEW {} by {by} (SELF-REVIEWED: no other session to continue)",
            short(combined)
        ),
    };
    log(&mut board, now, line);
    board.review = Some(review.clone());
    Ok(next(board, Outcome::ReviewStarted { review, resume: second }))
}

fn approve(mut board: Board, agent: &str, evidence: &str, now: u64) -> Result<Step, Error> {
    require_member(&board, agent)?;
    let review = board.review.as_mut().ok_or(Error::NoReview)?;
    if !review.reviewers.contains_key(agent) {
        return Err(Error::NotReviewer {
            agent: agent.to_string(),
        });
    }
    review
        .approved
        .insert(agent.to_string(), evidence.to_string());
    let complete = review.reviewers.keys().all(|r| review.approved.contains_key(r));
    review.landing = complete;
    let combined = review.combined.clone();
    log(&mut board, now, format!("APPROVE {agent} :: {evidence}"));
    Ok(next(
        board,
        Outcome::Approved {
            agent: agent.to_string(),
            complete,
            combined,
        },
    ))
}

/// The author's work goes back to it: its head no longer counts as finished, the review is
/// void, and its session (if concord can continue it, and the limit allows) gets the reason.
fn reject(
    mut board: Board,
    agent: &str,
    author: &str,
    reason: &str,
    now: u64,
) -> Result<Step, Error> {
    require_member(&board, agent)?;
    require_member(&board, author)?;
    let review = board.review.as_ref().ok_or(Error::NoReview)?;
    if !review.reviewers.contains_key(agent) {
        return Err(Error::NotReviewer {
            agent: agent.to_string(),
        });
    }
    if board.heads.remove(author).is_none() {
        return Err(Error::NotFinished {
            agent: author.to_string(),
        });
    }
    board.review = None;
    let count = {
        let count = board.rejections.entry(author.to_string()).or_default();
        *count += 1;
        *count
    };
    let blocked = count > REJECT_LIMIT;
    let session = board.plans.get(author).and_then(|p| p.session.clone());
    // The reviewer is running; an author that is the reviewer reads the reason itself.
    let resume = own_session(&board, author).filter(|_| !blocked && author != agent);
    let what = match (&resume, blocked) {
        (_, true) => format!("BLOCKED {author}: rejected {count} times"),
        (Some(s), _) => format!("RESUME {author} {}", s.id),
        (None, _) => format!("{author} fixes it (no session concord can continue)"),
    };
    log(
        &mut board,
        now,
        format!("REJECT {author} by {agent} :: {reason}; {what}"),
    );
    Ok(next(
        board,
        Outcome::Rejected {
            author: author.to_string(),
            reason: reason.to_string(),
            resume,
            blocked,
            session,
        },
    ))
}

/// The session concord may continue for `agent`: on this machine and not shared with another
/// team member. Subagents report their parent's session; a shared one belongs to an orchestrator
/// that is still running, and continuing it in parallel would fork its conversation.
fn own_session(board: &Board, agent: &str) -> Option<Session> {
    let session = board.plans.get(agent)?.session.clone()?;
    let shared = board.team.iter().any(|other| {
        other != agent
            && board
                .plans
                .get(other)
                .and_then(|p| p.session.as_ref())
                .is_some_and(|s| s.id == session.id)
    });
    (session.local && !shared).then_some(session)
}

/// What one reviewer checks, read from the board (`concord review`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assignment {
    pub combined: String,
    pub base: Option<String>,
    /// Per agent whose criteria this reviewer checks: its head, first and current criteria.
    pub criteria: Vec<(String, String, String, String)>,
    /// Contract changes to check the combined result against (empty unless `contracts_by`).
    pub contracts: Vec<Contract>,
    pub approved: bool,
}

pub fn assignment(board: &Board, reviewer: &str) -> Option<Assignment> {
    let review = board.review.as_ref()?;
    let agents = review.reviewers.get(reviewer)?;
    let criteria = agents
        .iter()
        .map(|a| {
            let versions = board.plans.get(a).map(|p| p.criteria.clone()).unwrap_or_default();
            let first = versions.first().cloned().unwrap_or_default();
            let current = versions.last().cloned().unwrap_or_default();
            let head = board.heads.get(a).cloned().unwrap_or_default();
            (a.clone(), head, first, current)
        })
        .collect();
    let contracts = if review.contracts_by == reviewer {
        board.history.values().flatten().cloned().collect()
    } else {
        Vec::new()
    };
    Some(Assignment {
        combined: review.combined.clone(),
        base: board.landing.as_ref().map(|l| l.base.clone()),
        criteria,
        contracts,
        approved: review.approved.contains_key(reviewer),
    })
}

/// Distinct finished heads, sorted: the changes the gate combines.
pub fn finished_heads(board: &Board) -> Vec<String> {
    let mut heads: Vec<String> = board.heads.values().cloned().collect();
    heads.sort();
    heads.dedup();
    heads
}

/// Board overview for `concord status`: team, planned agents, per-function holder and queue,
/// the change each finished agent stopped at, and how long ago each agent last ran a command.
pub fn status_lines(board: &Board, now: u64) -> Vec<String> {
    let planned: Vec<String> = board
        .team
        .iter()
        .filter(|a| board.plans.contains_key(*a))
        .cloned()
        .collect();
    let finished: Vec<String> = board
        .team
        .iter()
        .filter_map(|a| board.heads.get(a).map(|c| format!("{a}@{}", short(c))))
        .collect();
    let mut lines = vec![
        format!("team: {}", list_or_none(&board.team)),
        format!("planned: {}", list_or_none(&planned)),
    ];
    for (func, agents) in &board.queue {
        let holder = agents.first().map_or("none", String::as_str);
        let rest = agents.get(1..).unwrap_or_default();
        lines.push(format!(
            "fn {func}: holder={holder} queue={}",
            list_or_none(rest)
        ));
    }
    lines.push(format!("finished: {}", list_or_none(&finished)));
    if let Some(review) = &board.review {
        let waiting: Vec<String> = review
            .reviewers
            .keys()
            .filter(|r| !review.approved.contains_key(*r))
            .cloned()
            .collect();
        lines.push(format!(
            "review of {}: waiting for {}",
            short(&review.combined),
            list_or_none(&waiting)
        ));
    }
    let active: Vec<String> = board
        .team
        .iter()
        .map(|a| {
            let last = board.active.get(a).copied().unwrap_or(board.started);
            format!("{a} {}m ago", now.saturating_sub(last) / 60)
        })
        .collect();
    lines.push(format!("last command: {}", active.join(", ")));
    lines
}

fn next(board: Board, outcome: Outcome) -> Step {
    Step::Next(Box::new(board), outcome)
}

fn require_member(board: &Board, agent: &str) -> Result<(), Error> {
    if board.team.iter().any(|a| a == agent) {
        Ok(())
    } else {
        Err(Error::NotInTeam {
            agent: agent.to_string(),
            team: board.team.clone(),
        })
    }
}

fn all_planned(board: &Board) -> bool {
    board.team.iter().all(|a| board.plans.contains_key(a))
}

fn history<'a>(board: &'a Board, func: &str) -> &'a [Contract] {
    board
        .history
        .get(func)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn mark_seen(board: &mut Board, agent: &str, ids: Vec<String>) {
    let seen = board.seen.entry(agent.to_string()).or_default();
    for id in ids {
        if !seen.contains(&id) {
            seen.push(id);
        }
    }
}

fn log(board: &mut Board, now: u64, msg: String) {
    board.log.push(format!("{} {msg}", hms(now)));
}

/// UTC wall clock `HH:MM:SS` of Unix seconds `t`.
pub fn hms(t: u64) -> String {
    let day = t % 86_400;
    format!("{:02}:{:02}:{:02}", day / 3600, day % 3600 / 60, day % 60)
}

/// Python list repr (`['a', 'b']`), so log lines read the same as the prototype's.
pub fn py_list(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|s| format!("'{s}'")).collect();
    format!("[{}]", quoted.join(", "))
}

/// `none` for an empty list, the list repr otherwise (prototype: `users or 'none'`).
pub fn list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        py_list(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type T = Result<(), String>;
    const NOW: u64 = 12 * 3600;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn run(board: Option<Board>, cmd: Command) -> Result<Step, Error> {
        apply(board, &cmd, NOW)
    }

    /// Apply a command that must make progress; return the new board and outcome.
    fn step(board: &Board, cmd: Command) -> Result<(Board, Outcome), String> {
        match run(Some(board.clone()), cmd.clone()) {
            Ok(Step::Next(b, o)) => Ok((*b, o)),
            other => Err(format!("{cmd:?} did not progress: {other:?}")),
        }
    }

    fn init(team: &[&str]) -> Result<Board, String> {
        match run(
            None,
            Command::Init {
                team: s(team),
                landing: None,
            },
        ) {
            Ok(Step::Next(b, _)) => Ok(*b),
            other => Err(format!("init failed: {other:?}")),
        }
    }

    fn plan(agent: &str, modifies: &[&str], uses: &[&str]) -> Command {
        Command::Plan {
            agent: agent.into(),
            modifies: s(modifies),
            uses: s(uses),
            criteria: format!("{agent}'s task works"),
            session: None,
        }
    }
    /// A plan from a Claude Code session `id` on the board's machine.
    fn plan_in(agent: &str, id: &str, criteria: &str) -> Command {
        Command::Plan {
            agent: agent.into(),
            modifies: Vec::new(),
            uses: Vec::new(),
            criteria: criteria.into(),
            session: Some(Session {
                harness: Harness::Claude,
                id: id.into(),
                local: true,
            }),
        }
    }
    fn claim(agent: &str, func: &str) -> Command {
        Command::Claim {
            agent: agent.into(),
            func: func.into(),
        }
    }
    fn done(agent: &str, func: &str, change: &str, note: &str) -> Command {
        Command::Done {
            agent: agent.into(),
            func: func.into(),
            change: change.into(),
            note: note.into(),
        }
    }
    fn finish(agent: &str, change: &str) -> Command {
        Command::Finish {
            agent: agent.into(),
            change: change.into(),
        }
    }

    #[test]
    fn status_lines_show_holder_queue_and_finished() -> T {
        let b = init(&["a", "b"])?;
        let (b, _) = step(&b, plan("a", &["total"], &[]))?;
        let (b, _) = step(&b, plan("b", &["total"], &[]))?;
        let (b, _) = step(&b, finish("a", "c1"))?;
        let want = s(&[
            "team: ['a', 'b']",
            "planned: ['a', 'b']",
            "fn total: holder=a queue=['b']",
            "finished: ['a@c1']",
            "last command: a 2m ago, b 2m ago",
        ]);
        let got = status_lines(&b, NOW + 150);
        if got == want {
            Ok(())
        } else {
            Err(format!("{got:?}"))
        }
    }

    #[test]
    fn status_lines_before_plans() -> T {
        let b = init(&["a"])?;
        let want = s(&[
            "team: ['a']",
            "planned: none",
            "finished: none",
            "last command: a 0m ago",
        ]);
        let got = status_lines(&b, NOW);
        if got == want {
            Ok(())
        } else {
            Err(format!("{got:?}"))
        }
    }

    /// The cart scenario: A renames total, B changes total, C calls total.
    fn cart_planned() -> Result<Board, String> {
        let b = init(&["agent-a", "agent-b", "agent-c"])?;
        let (b, _) = step(&b, plan("agent-a", &["total"], &[]))?;
        let (b, _) = step(&b, plan("agent-b", &["total"], &[]))?;
        let (b, _) = step(&b, plan("agent-c", &[], &["total", "item_count"]))?;
        Ok(b)
    }

    /// `init` with no agents makes the folder a project: a board with its base and no round.
    #[test]
    fn init_without_agents_records_the_base_only() -> T {
        let landing = Some(Landing { base: "base0".into() });
        let (b, _) = match run(None, Command::Init { team: vec![], landing }) {
            Ok(Step::Next(b, o)) => (b, o),
            other => return Err(format!("no board: {other:?}")),
        };
        assert!(b.team.is_empty());
        assert_eq!(b.landing.as_ref().map(|l| l.base.as_str()), Some("base0"));
        Ok(())
    }

    #[test]
    fn init_resets_an_existing_board() -> T {
        let (b, _) = step(
            &cart_planned()?,
            Command::Init {
                team: s(&["x"]),
                landing: None,
            },
        )?;
        assert_eq!(b.team, s(&["x"]));
        assert!(b.plans.is_empty() && b.queue.is_empty());
        assert_eq!(b.log, s(&["12:00:00 INIT team=x"]));
        Ok(())
    }

    #[test]
    fn commands_before_init_fail() {
        assert_eq!(run(None, claim("a", "f")), Err(Error::NoBoard));
    }

    #[test]
    fn unknown_agent_is_rejected() -> T {
        let b = init(&["agent-a"])?;
        assert!(matches!(
            run(Some(b), plan("agenta", &["f"], &[])),
            Err(Error::NotInTeam { .. })
        ));
        Ok(())
    }

    #[test]
    fn queue_order_is_plan_order() -> T {
        let b = cart_planned()?;
        assert_eq!(b.queue.get("total"), Some(&s(&["agent-a", "agent-b"])));
        Ok(())
    }

    #[test]
    fn replanning_keeps_place_and_adds_no_duplicate_slot() -> T {
        let b = init(&["a", "b"])?;
        let (b, _) = step(&b, plan("a", &["f"], &["g"]))?;
        let (b, _) = step(&b, plan("b", &["f"], &[]))?;
        let (b, _) = step(&b, plan("a", &["f"], &[]))?;
        assert_eq!(b.queue.get("f"), Some(&s(&["a", "b"])));
        assert_eq!(b.plans.get("a").map(|p| p.uses.clone()), Some(vec![]));
        Ok(())
    }

    #[test]
    fn replanning_drops_slots_no_longer_modified() -> T {
        let b = init(&["a", "b"])?;
        let (b, _) = step(&b, plan("a", &["f"], &[]))?;
        let (b, _) = step(&b, plan("b", &["f"], &[]))?;
        let (b, _) = step(&b, plan("a", &["h"], &[]))?;
        assert_eq!(b.queue.get("f"), Some(&s(&["b"])));
        assert_eq!(b.queue.get("h"), Some(&s(&["a"])));
        Ok(())
    }

    #[test]
    fn claim_waits_for_every_agent_to_plan() -> T {
        let b = init(&["a", "b"])?;
        let (b, _) = step(&b, plan("a", &["f"], &[]))?;
        assert_eq!(run(Some(b), claim("a", "f")), Ok(Step::Wait));
        Ok(())
    }

    #[test]
    fn claim_waits_for_turn_then_shows_latest_contract() -> T {
        let b = cart_planned()?;
        assert_eq!(
            run(Some(b.clone()), claim("agent-b", "total")),
            Ok(Step::Wait)
        );
        let (b, o) = step(&b, claim("agent-a", "total"))?;
        assert_eq!(
            o,
            Outcome::Claimed {
                func: "total".into(),
                latest: None
            }
        );
        let (b, _) = step(&b, done("agent-a", "total", "ch-a", "renamed to subtotal"))?;
        let (b, o) = step(&b, claim("agent-b", "total"))?;
        let Outcome::Claimed {
            latest: Some(c), ..
        } = o
        else {
            return Err(format!("expected latest contract, got {o:?}"));
        };
        assert_eq!((c.id.as_str(), c.change.as_str()), ("total#1", "ch-a"));
        assert_eq!(b.seen.get("agent-b"), Some(&s(&["total#1"])));
        Ok(())
    }

    #[test]
    fn claim_of_unplanned_function_fails_after_barrier() -> T {
        let b = cart_planned()?;
        assert!(matches!(
            run(Some(b), claim("agent-c", "total")),
            Err(Error::NotPlannedFunc { .. })
        ));
        Ok(())
    }

    #[test]
    fn done_by_non_holder_fails() -> T {
        let b = cart_planned()?;
        assert!(matches!(
            run(Some(b), done("agent-b", "total", "x", "n")),
            Err(Error::NotHolder { .. })
        ));
        Ok(())
    }

    #[test]
    fn done_passes_function_and_names_callers() -> T {
        let (b, _) = step(&cart_planned()?, claim("agent-a", "total"))?;
        let (b, o) = step(&b, done("agent-a", "total", "ch-a", "renamed"))?;
        let released = Outcome::Released {
            func: "total".into(),
            next: Some("agent-b".into()),
            callers: s(&["agent-c"]),
        };
        assert_eq!(o, released);
        assert_eq!(
            b.log.last().map(String::as_str),
            Some("12:00:00 DONE agent-a total ch-a :: renamed (callers to re-check: ['agent-c'])")
        );
        Ok(())
    }

    #[test]
    fn finish_waits_while_used_function_is_queued_by_others() -> T {
        let b = cart_planned()?;
        assert_eq!(
            run(Some(b.clone()), finish("agent-c", "ch-c")),
            Ok(Step::Wait)
        );
        let (b, _) = step(&b, claim("agent-a", "total"))?;
        let (b, _) = step(&b, done("agent-a", "total", "ch-a", "renamed"))?;
        assert_eq!(run(Some(b), finish("agent-c", "ch-c")), Ok(Step::Wait));
        Ok(())
    }

    #[test]
    fn finish_reports_unseen_contracts_once() -> T {
        let (b, _) = step(&cart_planned()?, claim("agent-a", "total"))?;
        let (b, _) = step(&b, done("agent-a", "total", "ch-a", "renamed"))?;
        let (b, _) = step(&b, claim("agent-b", "total"))?;
        let (b, _) = step(&b, done("agent-b", "total", "ch-b", "float"))?;
        let (b, o) = step(&b, finish("agent-c", "ch-c"))?;
        let Outcome::Finished { unseen, .. } = o else {
            return Err(format!("expected finished, got {o:?}"));
        };
        let ids: Vec<&str> = unseen.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["total#1", "total#2"]);
        assert_eq!(
            b.log.last().map(String::as_str),
            Some("12:00:00 FINISH agent-c ch-c new_contracts=['total#1', 'total#2']")
        );
        let (b, o) = step(&b, finish("agent-c", "ch-c2"))?;
        assert!(matches!(o, Outcome::Finished { ref unseen, .. } if unseen.is_empty()));
        assert_eq!(b.heads.get("agent-c").map(String::as_str), Some("ch-c2"));
        Ok(())
    }

    #[test]
    fn finish_of_own_function_does_not_wait_for_self() -> T {
        let b = init(&["a"])?;
        let (b, _) = step(&b, plan("a", &["f"], &["f"]))?;
        assert!(matches!(
            run(Some(b), finish("a", "ch")),
            Ok(Step::Next(..))
        ));
        Ok(())
    }

    #[test]
    fn finish_waits_until_the_agent_itself_planned() -> T {
        let b = init(&["a"])?;
        assert_eq!(run(Some(b), finish("a", "ch")), Ok(Step::Wait));
        Ok(())
    }

    const LIMIT: u64 = SILENCE_LIMIT;

    #[test]
    fn waiting_on_an_active_holder_is_not_stalled() -> T {
        let (b, _) = step(&cart_planned()?, claim("agent-a", "total"))?;
        let waiting = claim("agent-b", "total");
        assert_eq!(run(Some(b.clone()), waiting.clone()), Ok(Step::Wait));
        assert_eq!(stalled(&b, &waiting, NOW + LIMIT, LIMIT), None);
        Ok(())
    }

    #[test]
    fn a_holder_silent_past_the_limit_stalls_its_waiters() -> T {
        let (b, _) = step(&cart_planned()?, claim("agent-a", "total"))?;
        let later = NOW + LIMIT + 60;
        assert_eq!(
            stalled(&b, &claim("agent-b", "total"), later, LIMIT),
            Some(("agent-a".to_string(), LIMIT + 60))
        );
        // agent-c uses total, which agent-a holds and agent-b waits for: both block it.
        assert_eq!(
            stalled(&b, &finish("agent-c", "ch-c"), later, LIMIT).map(|(a, _)| a),
            Some("agent-a".to_string())
        );
        Ok(())
    }

    #[test]
    fn an_agent_that_never_planned_stalls_the_barrier() -> T {
        let b = init(&["a", "b"])?;
        let (b, _) = step(&b, plan("a", &["f"], &[]))?;
        let silent_b = stalled(&b, &claim("a", "f"), NOW + LIMIT + 1, LIMIT);
        assert_eq!(silent_b.map(|(a, _)| a), Some("b".to_string()));
        Ok(())
    }

    #[test]
    fn release_passes_held_functions_on_and_unblocks_the_team() -> T {
        let (b, _) = step(&cart_planned()?, claim("agent-a", "total"))?;
        let release = Command::Release {
            agent: "agent-a".into(),
        };
        let (b, outcome) = step(&b, release)?;
        assert_eq!(
            outcome,
            Outcome::Removed {
                agent: "agent-a".into(),
                next: vec![("total".into(), "agent-b".into())],
            }
        );
        assert_eq!(b.team, s(&["agent-b", "agent-c"]));
        let (_, claimed) = step(&b, claim("agent-b", "total"))?;
        assert!(matches!(claimed, Outcome::Claimed { .. }));
        Ok(())
    }

    fn add(agent: &str) -> Command {
        Command::Add {
            agent: agent.into(),
        }
    }

    #[test]
    fn an_added_agent_queues_last_and_holds_the_barrier_until_it_plans() -> T {
        let (b, added) = step(&cart_planned()?, add("agent-d"))?;
        assert_eq!(
            added,
            Outcome::Added {
                agent: "agent-d".into()
            }
        );
        assert_eq!(
            run(Some(b.clone()), claim("agent-a", "total")),
            Ok(Step::Wait)
        );
        let (b, _) = step(&b, plan("agent-d", &["total"], &[]))?;
        assert_eq!(
            b.queue.get("total"),
            Some(&s(&["agent-a", "agent-b", "agent-d"]))
        );
        let (_, claimed) = step(&b, claim("agent-a", "total"))?;
        assert!(matches!(claimed, Outcome::Claimed { .. }));
        Ok(())
    }

    #[test]
    fn adding_a_member_fails_and_a_released_one_can_come_back() -> T {
        let b = cart_planned()?;
        assert_eq!(
            run(Some(b.clone()), add("agent-a")),
            Err(Error::AlreadyInTeam {
                agent: "agent-a".into()
            })
        );
        let release = Command::Release {
            agent: "agent-a".into(),
        };
        let (b, _) = step(&b, release)?;
        let (b, _) = step(&b, add("agent-a"))?;
        assert!(b.team.contains(&"agent-a".to_string()));
        assert!(!b.plans.contains_key("agent-a"));
        Ok(())
    }

    #[test]
    fn a_late_plan_names_finished_agents_using_what_it_modifies() -> T {
        let b = init(&["agent-a", "agent-c"])?;
        let (b, _) = step(&b, plan("agent-a", &["total"], &[]))?;
        let (b, _) = step(&b, plan("agent-c", &[], &["item_count"]))?;
        let (b, _) = step(&b, finish("agent-c", "c1"))?;
        let (b, _) = step(&b, add("agent-d"))?;
        let (_, planned) = step(&b, plan("agent-d", &["item_count"], &[]))?;
        let Outcome::Planned { finished_users, .. } = planned else {
            return Err(format!("not planned: {planned:?}"));
        };
        assert_eq!(finished_users, s(&["agent-c"]));
        Ok(())
    }

    fn join(agent: &str) -> Command {
        Command::Join {
            agent: (!agent.is_empty()).then(|| agent.into()),
            landing: Some(Landing {
                base: "base1".into(),
            }),
        }
    }

    #[test]
    fn the_first_join_starts_the_board_and_later_ones_join_the_team() -> T {
        let started = run(None, join("a"));
        let Ok(Step::Next(b, outcome)) = started else {
            return Err(format!("no board: {started:?}"));
        };
        assert_eq!(
            outcome,
            Outcome::Joined {
                agent: "a".into(),
                joined: true
            }
        );
        let (b, _) = step(&b, join("b"))?;
        assert_eq!(b.team, s(&["a", "b"]));
        let (again, outcome) = step(&b, join("a"))?;
        assert_eq!(again.team, b.team);
        assert_eq!(
            outcome,
            Outcome::Joined {
                agent: "a".into(),
                joined: false
            }
        );
        Ok(())
    }

    /// `agent` (the round's only reviewer) starts the review of `combined` and approves it: the
    /// review is landing.
    fn approve_alone(b: &Board, agent: &str, combined: &str) -> Result<Board, String> {
        let start = Command::StartReview { by: agent.into(), combined: combined.into() };
        let (b, _) = step(b, start)?;
        let (b, _) = step(&b, Command::Approve { agent: agent.into(), evidence: "ok".into() })?;
        Ok(b)
    }

    fn landed(combined: &str) -> Command {
        Command::Landed { combined: combined.into() }
    }

    #[test]
    fn landing_ends_the_round_for_the_landed_and_keeps_the_rest() -> T {
        let started = run(None, join("a"));
        let Ok(Step::Next(b, _)) = started else {
            return Err(format!("no board: {started:?}"));
        };
        let (b, _) = step(&b, plan("a", &[], &[]))?;
        let (b, _) = step(&b, finish("a", "a1"))?;
        let b = approve_alone(&b, "a", "land1")?;
        // b joins while a's round lands: it is not in the review and keeps working.
        let (b, _) = step(&b, join("b"))?;
        let (b, _) = step(&b, plan("b", &["f"], &[]))?;
        let (b, _) = step(&b, landed("land1"))?;
        assert_eq!(b.team, s(&["b"]));
        assert_eq!(b.queue.get("f"), Some(&s(&["b"])));
        assert_eq!(b.landing.as_ref().map(|l| l.base.as_str()), Some("land1"));
        let (b, _) = step(&b, finish("b", "b1"))?;
        let b = approve_alone(&b, "b", "land2")?;
        let (b, _) = step(&b, landed("land2"))?;
        assert!(b.team.is_empty() && b.plans.is_empty() && b.history.is_empty());
        assert_eq!(b.landing.as_ref().map(|l| l.base.as_str()), Some("land2"));
        // The next round starts from the main folder as it is (`join` passes its snapshot,
        // base1 here); a join while a round runs (b above) kept the round's base.
        let (b, _) = step(&b, join("c"))?;
        assert_eq!(b.landing.as_ref().map(|l| l.base.as_str()), Some("base1"));
        Ok(())
    }

    #[test]
    fn while_a_round_lands_its_agents_wait_and_only_that_round_lands() -> T {
        let b = finished_pair("sa", "sb")?;
        let (b, _) = step(&b, Command::StartReview { by: "a".into(), combined: "c1".into() })?;
        let refused = run(Some(b.clone()), landed("c1"));
        assert_eq!(refused, Err(Error::NotLanding { combined: "c1".into() }), "not approved yet");
        let (b, _) = step(&b, Command::Approve { agent: "a".into(), evidence: "ok".into() })?;
        let (b, outcome) = step(&b, Command::Approve { agent: "b".into(), evidence: "ok".into() })?;
        assert_eq!(
            outcome,
            Outcome::Approved { agent: "b".into(), complete: true, combined: "c1".into() }
        );
        for cmd in [
            finish("b", "b2"),
            Command::Release { agent: "b".into() },
            Command::Approve { agent: "a".into(), evidence: "again".into() },
            Command::Reject { agent: "a".into(), author: "b".into(), reason: "late".into() },
        ] {
            assert_eq!(run(Some(b.clone()), cmd.clone()), Ok(Step::Wait), "{cmd:?}");
        }
        let other = run(Some(b.clone()), landed("c2"));
        assert_eq!(other, Err(Error::NotLanding { combined: "c2".into() }));
        let (b, _) = step(&b, landed("c1"))?;
        assert!(b.team.is_empty() && b.review.is_none());
        Ok(())
    }

    #[test]
    fn an_agent_without_a_name_gets_the_first_free_one() -> T {
        let started = run(None, join(""));
        let Ok(Step::Next(b, outcome)) = started else {
            return Err(format!("no board: {started:?}"));
        };
        assert_eq!(
            outcome,
            Outcome::Joined {
                agent: "agent-1".into(),
                joined: true
            }
        );
        let (b, _) = step(&b, join("agent-2"))?;
        let (b, _) = step(&b, join(""))?;
        assert_eq!(b.team, s(&["agent-1", "agent-2", "agent-3"]));
        Ok(())
    }

    #[test]
    fn names_that_cannot_be_folders_or_branches_are_refused() {
        for bad in ["a b", "a/b", "default", "concord-gate", ".x"] {
            let refused = run(None, join(bad));
            assert!(
                matches!(refused, Err(Error::BadName { .. })),
                "{bad:?}: {refused:?}"
            );
        }
        assert!(matches!(
            run(None, Command::Init { team: s(&["ok", "no/pe"]), landing: None }),
            Err(Error::BadName { .. })
        ));
    }

    #[test]
    fn a_joined_round_lands_when_everyone_finished_without_a_recheck() -> T {
        let started = run(None, join("a"));
        let Ok(Step::Next(b, _)) = started else {
            return Err(format!("no board: {started:?}"));
        };
        let (b, _) = step(&b, join("c"))?;
        let (b, _) = step(&b, plan("a", &["f"], &[]))?;
        let (b, _) = step(&b, plan("c", &[], &["f"]))?;
        let (b, _) = step(&b, claim("a", "f"))?;
        let (b, _) = step(&b, done("a", "f", "ch-a", "f() -> int; CHANGED: returns 10"))?;
        let (b, _) = step(&b, finish("a", "ch-a"))?;
        assert!(!review_due(&b), "c is still working");
        let (b, _) = step(&b, finish("c", "c1"))?;
        assert!(!review_due(&b), "c must adapt to a's change first (exit 10)");
        let (b, _) = step(&b, finish("c", "c2"))?;
        assert!(review_due(&b));
        Ok(())
    }

    #[test]
    fn an_init_round_is_reviewed_like_a_joined_one() -> T {
        let b = init(&["a"])?;
        let (b, _) = step(&b, plan("a", &[], &[]))?;
        let (b, _) = step(&b, finish("a", "a1"))?;
        assert!(review_due(&b));
        assert!(matches!(
            run(Some(b), plan_in("a", "s", "  ")),
            Err(Error::NoCriteria)
        ));
        Ok(())
    }

    /// a and b finished; a's finish completed the round (`by`).
    fn finished_pair(session_a: &str, session_b: &str) -> Result<Board, String> {
        let b = init(&["a", "b"])?;
        let (b, _) = step(&b, plan_in("a", session_a, "a's task works"))?;
        let (b, _) = step(&b, plan_in("b", session_b, "b's first idea"))?;
        let (b, _) = step(&b, plan_in("b", session_b, "b's task works"))?;
        let (b, _) = step(&b, finish("b", "b1"))?;
        let (b, _) = step(&b, finish("a", "a1"))?;
        Ok(b)
    }

    #[test]
    fn the_last_finisher_reviews_the_others_and_another_session_reviews_it() -> T {
        let b = finished_pair("sa", "sb")?;
        let (b, outcome) = step(&b, Command::StartReview { by: "a".into(), combined: "c1".into() })?;
        let Outcome::ReviewStarted { resume: Some((second, _)), .. } = outcome else {
            return Err(format!("no second reviewer: {outcome:?}"));
        };
        assert_eq!(second, "b", "b's own session reviews a's criteria");
        let a_task = assignment(&b, "a").ok_or("a reviews nothing")?;
        assert_eq!(a_task.criteria.len(), 1);
        assert_eq!(a_task.criteria[0].2, "b's first idea", "first stated criteria stay visible");
        assert_eq!(a_task.criteria[0].3, "b's task works");
        let (b, outcome) = step(&b, Command::Approve { agent: "a".into(), evidence: "ok".into() })?;
        assert_eq!(
            outcome,
            Outcome::Approved { agent: "a".into(), complete: false, combined: "c1".into() }
        );
        let (_, outcome) = step(&b, Command::Approve { agent: "b".into(), evidence: "ok".into() })?;
        assert_eq!(
            outcome,
            Outcome::Approved { agent: "b".into(), complete: true, combined: "c1".into() }
        );

        // Subagents report their parent's session: nobody else's session to continue.
        let shared = finished_pair("parent", "parent")?;
        let (shared, outcome) =
            step(&shared, Command::StartReview { by: "a".into(), combined: "c1".into() })?;
        assert!(matches!(outcome, Outcome::ReviewStarted { resume: None, .. }));
        let own = assignment(&shared, "a").ok_or("a reviews nothing")?;
        assert_eq!(own.criteria.len(), 2, "a checks its own criteria too");
        assert!(shared.log.iter().any(|l| l.contains("SELF-REVIEWED")));
        Ok(())
    }

    #[test]
    fn a_failed_combine_waits_until_a_head_changes() -> T {
        let b = finished_pair("sa", "sb")?;
        let (b, _) = step(&b, Command::GateFailed { reason: "unresolved conflicts: m.py".into() })?;
        assert_eq!(b.combine_failed.as_deref(), Some("unresolved conflicts: m.py"));
        assert!(!review_due(&b), "the same heads would fail again");
        let (b, _) = step(&b, finish("b", "b2"))?;
        assert_eq!(b.combine_failed, None);
        assert!(review_due(&b));
        Ok(())
    }

    #[test]
    fn a_rejection_sends_the_work_back_until_the_limit() -> T {
        let mut b = finished_pair("sa", "sb")?;
        for round in 1..=REJECT_LIMIT + 1 {
            let (next, _) = step(&b, Command::StartReview { by: "a".into(), combined: "c".into() })?;
            let reject = Command::Reject {
                agent: "a".into(),
                author: "b".into(),
                reason: "wrong".into(),
            };
            let (next, outcome) = step(&next, reject)?;
            let Outcome::Rejected { resume, blocked, .. } = outcome else {
                return Err(format!("not rejected: {outcome:?}"));
            };
            assert!(next.review.is_none() && !next.heads.contains_key("b"));
            assert_eq!(blocked, round > REJECT_LIMIT);
            assert_eq!(resume.is_some(), round <= REJECT_LIMIT, "b's session gets the reason");
            let (next, _) = step(&next, finish("b", &format!("b{}", round + 1)))?;
            b = next;
        }
        Ok(())
    }

    #[test]
    fn a_waiting_agent_stays_alive() -> T {
        let b = cart_planned()?;
        let later = NOW + SILENCE_LIMIT;
        let waiting = claim("agent-c", "total");
        let alive = heartbeat(Some(b), &waiting, later).ok_or("no heartbeat")?;
        assert_eq!(alive.active.get("agent-c"), Some(&later));
        assert_eq!(heartbeat(Some(alive.clone()), &waiting, later + 1), None, "just recorded");
        assert_eq!(heartbeat(Some(alive), &claim("stranger", "total"), later), None);
        Ok(())
    }

    #[test]
    fn commands_record_when_each_agent_was_last_active() -> T {
        let b = cart_planned()?;
        assert_eq!(b.active.get("agent-c"), Some(&NOW));
        assert_eq!(b.started, NOW);
        Ok(())
    }

    #[test]
    fn finished_heads_are_distinct_and_sorted() {
        let mut b = Board::default();
        b.heads.insert("a".into(), "zz".into());
        b.heads.insert("b".into(), "aa".into());
        b.heads.insert("c".into(), "zz".into());
        assert_eq!(finished_heads(&b), s(&["aa", "zz"]));
    }

    #[test]
    fn board_reads_prototype_state_json() -> T {
        let json = include_str!("../experiments/cart-scenario/evidence/v2-board-state.json");
        let b: Board = serde_json::from_str(json).map_err(|e| e.to_string())?;
        assert_eq!(b.history.get("total").map(Vec::len), Some(2));
        assert_eq!(b.heads.len(), 3);
        Ok(())
    }
}
