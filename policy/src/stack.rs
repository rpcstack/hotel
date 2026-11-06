//! SLOT 2: the policy-module stack and the hooks type of every generated stub.
//!
//! `Stack` lists the modules that run in the hooks of every gRPC client and
//! server stub: in list order in pre-hooks, in reverse order in post-hooks. It
//! starts EMPTY (a pass-through), so the `policy` mode behaves like `baseline`.
//! `Hooks` is the policy crate's own type (never `NoopHooks`), even while it
//! does nothing; it is the name the hotel crate's `build.rs` refers to.
//!
//! To add a policy, list modules: `policy_stack![A, B, ...]`, where each is a type
//! implementing `rpcstack::Module`. rpcstack's published policies (the
//! `rpcstack/` submodule's `examples/`: masa, breakwater, dagor, rajomon,
//! c3, ...) show modules and whole stacks, e.g. `examples/masa/masa-policy/src/masa_stack.rs`;
//! `docs/PAPERS.md` and `docs/VIBESYS.md` explain them. A module an example
//! crate defines needs that crate as a dependency in this crate's `Cargo.toml`
//! (a git dependency at the same rev as the framework).

use rpcstack::policy_stack;

/// The module stack: empty.
pub type Stack = policy_stack![];

/// The hooks every generated client and server stub runs.
pub type Hooks = rpcstack_tonic::PolicyHooks<Stack>;
