//! SLOT 1: the run-queue scheduler.
//!
//! This file is compiled twice from one source: into `rpcstack-sched` as the
//! run queue of every Tokio worker (selected through `RPCSTACK_QUEUE_SOURCE`,
//! see `.cargo/config.toml`), and into this crate (`hotel_policy::queue`) so it
//! can be unit-tested with `cargo test -p hotel-policy`. Only use items that
//! exist at the root of both crates: `crate::{RunQueue, TaskView, Meta,
//! PollOutcome, SchedFlavor, default_policy}`; `lib.rs` re-exports these.
//!
//! It starts as the priority heap Masa's `sched_slo` selects today
//! (`rpcstack_sched::builtin::prio_heap::BinaryHeapQueue`): smaller `Meta`
//! first, ties in heap order, with infrastructure work (`Meta` 0) in a FIFO
//! lane drained first. `Meta` is the priority the policy stack set for the
//! request; the stack decides what it means (for Masa: the request's deadline
//! in microseconds, so this is earliest-deadline-first).

use std::collections::{BinaryHeap, VecDeque};

#[path = "keyed.rs"]
mod keyed;
use crate::{default_policy, RunQueue, SchedFlavor, TaskView};
use keyed::Keyed;

/// Marker that proves which source `rpcstack-sched` compiled; checked by
/// `tests/wiring.rs`. Keep it.
pub const POLICY_QUEUE_ID: &str = "hotel-policy/queue/queue.rs";

/// Priority-heap queue with a FIFO lane for infrastructure work.
pub struct Queue<T> {
    q: BinaryHeap<Keyed<T>>,
    infra_q: VecDeque<T>,
}

impl<T> RunQueue<T> for Queue<T> {
    const FLAVOR: SchedFlavor = SchedFlavor::Prio;

    fn with_capacity(cap: usize) -> Self {
        Self {
            q: BinaryHeap::with_capacity(cap),
            infra_q: VecDeque::new(),
        }
    }

    fn push(&mut self, item: T, view: &TaskView<'_>) {
        if default_policy::is_infra(view.meta) {
            self.infra_q.push_back(item);
        } else {
            self.q.push(Keyed::new(*view.meta, item));
        }
    }

    fn pop(&mut self) -> Option<T> {
        if let Some(item) = self.infra_q.pop_front() {
            return Some(item);
        }
        self.q.pop().map(|keyed| keyed.item)
    }

    fn len(&self) -> usize {
        self.q.len() + self.infra_q.len()
    }

    fn capacity(&self) -> usize {
        self.q.capacity() + self.infra_q.capacity()
    }
}

// Tests compile only in this crate (`cargo test -p hotel-policy`); inside
// rpcstack-sched the `test` cfg is never set for this module.
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

    fn drain(q: &mut Queue<&'static str>) -> Vec<&'static str> {
        std::iter::from_fn(|| q.pop()).collect()
    }

    #[test]
    fn smaller_meta_first() {
        let mut q = Queue::with_capacity(4);
        push(&mut q, "late", 300);
        push(&mut q, "early", 100);
        push(&mut q, "mid", 200);
        assert_eq!(q.len(), 3);
        assert_eq!(drain(&mut q), ["early", "mid", "late"]);
        assert!(q.is_empty());
    }

    #[test]
    fn infrastructure_work_runs_first_in_fifo_order() {
        let mut q = Queue::with_capacity(4);
        push(&mut q, "request", 1);
        push(&mut q, "infra-a", 0);
        push(&mut q, "infra-b", 0);
        assert_eq!(drain(&mut q), ["infra-a", "infra-b", "request"]);
    }

    #[test]
    fn flavor_is_priority() {
        assert_eq!(<Queue<u64> as RunQueue<u64>>::FLAVOR, SchedFlavor::Prio);
    }

    /// The policy queue must order exactly like the built-in queue Masa
    /// selects (the initial content's contract). Delete this test when you
    /// change the ordering on purpose.
    #[test]
    fn initial_content_matches_masa_builtin_queue() {
        use rpcstack_sched::builtin::prio_heap::BinaryHeapQueue;
        let metas = [5u64, 0, 9, 5, 2, 0, 9, 1, 5, 7, 7, 3];
        let mut ours = Queue::with_capacity(0);
        let mut theirs = BinaryHeapQueue::with_capacity(0);
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
