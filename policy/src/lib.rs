//! The hotel app's RPC policy. This crate is the only place an optimization
//! agent edits; see `README.md` in this directory.
//!
//! Two slots:
//! - [`queue`] (`../queue/queue.rs`): the run-queue scheduler, compiled into
//!   `rpcstack-sched` via `RPCSTACK_QUEUE_SOURCE` (`sched_custom`).
//! - [`stack`] (`stack.rs`): the policy-module stack and the [`Hooks`] type
//!   every generated gRPC stub uses (`default_hooks_path("hotel_policy::Hooks")`).

// The queue source is shared with rpcstack-sched, whose crate root exports
// these; re-exporting them here makes the same file compile in both crates.
pub use rpcstack_sched::{default_policy, Meta, PollOutcome, RunQueue, SchedFlavor, TaskView};

#[path = "../queue/queue.rs"]
pub mod queue;
pub mod stack;

pub use stack::{Hooks, Stack};
