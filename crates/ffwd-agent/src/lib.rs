//! Agent workspaces (EXPERIMENTAL, off by default).
//!
//! A run gives a coding agent its own Git worktree on a new branch, starts
//! it inside a sandbox, and leaves the result as normal commits that the
//! user reviews like a pull request. Nothing here touches the user's
//! checkout: the agent works in the worktree, and merging or discarding is
//! a separate, confirmed step (AGENTS.md rule 8).
//!
//! Everything runs on worker threads; the UI receives [`Event`]s.
//! Design: `docs/decisions/0002-agent-workspaces.md`; every setting:
//! `docs/AGENTS-EXPERIMENTAL.md`.

pub mod config;
pub mod proc;
pub mod record;
pub mod run;
pub mod sandbox;

pub use config::{Config, GitAccess, HomeAccess, SandboxKind};
pub use record::{Record, Status};
pub use run::{Event, EventKind, Handle};
