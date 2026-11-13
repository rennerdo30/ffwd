//! Repository model for ffwd. Everything here runs on worker threads; the UI
//! only receives finished values.
//!
//! The proof of concept reads through the `git` CLI (the same semantics as
//! the user's own Git, hooks and config). The backend decision between the
//! CLI, gitoxide and libgit2 is open (docs/SPEC.md §4) and will be measured.

pub mod changes;
pub mod conflict;
pub mod diff;
pub mod files;
pub mod git;
pub mod graph;
pub mod log;
pub mod ops;
pub mod rebase;
pub mod refops;
pub mod refs;
pub mod remotes;
pub mod search;
pub mod stash;

pub use git::{Git, GitError};
pub use graph::{Graph, GraphRow, Seg};
pub use log::{Commit, Oid};
