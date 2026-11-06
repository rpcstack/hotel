//! SLOT 1: the run-queue scheduler.
//!
//! This file is compiled twice from one source: into `rpcstack-sched` as the
//! run queue of every Tokio worker (selected through `RPCSTACK_QUEUE_SOURCE`,
//! see `.cargo/config.toml`), and into this crate (`hotel_policy::queue`) so it
//! can be unit-tested with `cargo test -p hotel-policy`. Only use items that
//! exist at the root of both crates: `crate::{RunQueue, TaskView, Meta,
//! PollOutcome, SchedFlavor, default_policy}`; `lib.rs` re-exports these.
//! Relative submodules (`#[path = "x.rs"] mod x;`) resolve next to this file.
//!
//! It starts as a no-op policy: plain first-in-first-out, ignoring `Meta`, the
//! same as the framework's default queue (`rpcstack_sched::builtin::fifo`) that
//! the `baseline` mode runs. For queues that do something, see the `rpcstack/`
//! submodule: `libs/rpcstack-sched/src/custom.rs` (priority heap),
//! `examples/tailclipper/queue.rs`, and the queues the other `examples/` use.

use std::collections::VecDeque;

use crate::{RunQueue, SchedFlavor, TaskView};

/// Marker that proves which source `rpcstack-sched` compiled; checked by
/// `tests/wiring.rs`. Keep it.
pub const POLICY_QUEUE_ID: &str = "hotel-policy/queue/queue.rs";

/// First-in-first-out queue.
pub struct Queue<T> {
    inner: VecDeque<T>,
}

impl<T> RunQueue<T> for Queue<T> {
    const FLAVOR: SchedFlavor = SchedFlavor::Fifo;

    fn with_capacity(cap: usize) -> Self {
        Self {
            inner: VecDeque::with_capacity(cap),
        }
    }

    fn push(&mut self, item: T, _view: &TaskView<'_>) {
        self.inner.push_back(item);
    }

    fn pop(&mut self) -> Option<T> {
        self.inner.pop_front()
    }

    fn len(&self) -> usize {
        self.inner.len()
    }

    fn capacity(&self) -> usize {
        self.inner.capacity()
    }
}

// Tests compile only in this crate (`cargo test -p hotel-policy`).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Meta;
    use std::time::Instant;

    fn push(q: &mut Queue<&'static str>, item: &'static str, meta: u64) {
        let m = Meta::new(meta);
        let now = Instant::now();
        q.push(item, &TaskView::new(1, &m, now, now, 0));
    }

    #[test]
    fn arrival_order_whatever_the_meta() {
        let mut q = Queue::with_capacity(4);
        push(&mut q, "a", 300);
        push(&mut q, "b", 0);
        push(&mut q, "c", 100);
        assert_eq!(q.len(), 3);
        let order: Vec<_> = std::iter::from_fn(|| q.pop()).collect();
        assert_eq!(order, ["a", "b", "c"]);
        assert!(q.is_empty());
    }

    #[test]
    fn flavor_is_fifo() {
        assert_eq!(<Queue<u64> as RunQueue<u64>>::FLAVOR, SchedFlavor::Fifo);
    }

    /// The initial content must order exactly like the framework's default
    /// queue (what `baseline` runs). Replace this test when you change the
    /// ordering on purpose.
    #[test]
    fn initial_content_matches_builtin_fifo_queue() {
        use rpcstack_sched::builtin::fifo::FifoQueue;
        let metas = [5u64, 0, 9, 5, 2, 0, 9, 1, 5, 7, 7, 3];
        let mut ours = Queue::with_capacity(0);
        let mut theirs = FifoQueue::with_capacity(0);
        let now = Instant::now();
        for (i, m) in metas.iter().enumerate() {
            let meta = Meta::new(*m);
            let view = TaskView::new(i as u64, &meta, now, now, 0);
            ours.push(i, &view);
            theirs.push(i, &view);
        }
        let a: Vec<usize> = std::iter::from_fn(|| ours.pop()).collect();
        let b: Vec<usize> = std::iter::from_fn(|| theirs.pop()).collect();
        assert_eq!(a, b);
    }
}
