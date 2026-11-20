//! concord: coordination board and agent-reviewed landing for parallel coding agents.
//! Protocol and architecture: docs/design.md.

/// `println!` that never panics: a reader that went away (`concord log | head -1`) must not
/// abort a command halfway through a state change, so a failed write to stdout is dropped and
/// the command finishes with its own exit code.
macro_rules! out {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), $($arg)*);
    }};
}

/// `eprintln!` that never panics, for the same reason as `out!`.
macro_rules! err_out {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($arg)*);
    }};
}

mod cli;
mod core;
mod diff;
mod gate;
pub mod merge;
pub mod objects;
mod remote;
mod serve;
mod store;
mod team;
#[cfg(test)]
mod test_util;
pub mod worktree;

pub use cli::main;
