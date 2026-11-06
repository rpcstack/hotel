//! Wiring of the two slots. These fail if a slot silently falls back to a
//! framework default.

use std::any::TypeId;

/// `rpcstack-sched` compiled *this* policy's queue, not the framework's
/// editable `custom.rs` nor another source (unset/overridden
/// `RPCSTACK_QUEUE_SOURCE`). `external` only exists when a source was supplied.
#[test]
fn run_queue_is_the_policy_queue() {
    assert_eq!(
        rpcstack_sched::external::POLICY_QUEUE_ID,
        hotel_policy::queue::POLICY_QUEUE_ID
    );
    assert_eq!(
        TypeId::of::<rpcstack_sched::SelectedQueue<u64>>(),
        TypeId::of::<rpcstack_sched::external::Queue<u64>>(),
    );
}

/// The hooks are a real policy stack, not the no-op hooks.
#[test]
fn hooks_are_not_noop() {
    assert_ne!(
        TypeId::of::<hotel_policy::Hooks>(),
        TypeId::of::<tonic::rpcstack::noop::NoopHooks>()
    );
    let name = std::any::type_name::<hotel_policy::Stack>();
    assert!(name.contains("BudgetModule"), "{name}");
}
