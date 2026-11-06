//! SLOT 2: the policy-module stack and the hooks type of every generated stub.
//!
//! `Stack` lists the modules in the order they run in pre-hooks (and in the
//! reverse order in post-hooks): budget -> deadline guard -> estimation ->
//! admission. The budget module must stay first: every other module reads the
//! budget it opens, and the framework seals each child's budget after all
//! modules ran. This is exactly Masa's `MasaStack` for the feature set
//! `sched_slo,abort_slo,signal_slack,est_mean_var,ac_pred` (its other slots,
//! oracle and queue-latency, are disabled there and omitted here).
//!
//! To change the policy: add, remove or reorder modules, or write a module
//! implementing `masa_policy::Module` and put it in the list. See
//! rpcstack's `docs/POLICY_MODULES.md` and `README.md` in this crate's
//! directory. Keep `Hooks` as the name `build.rs` of the hotel crate refers to.

use masa_policy::modules::{
    BudgetModule, E2eDeadlineGuardModule, EstimationModule, PredAdmissionModule,
};
use masa_policy::policy_stack;

/// The module stack.
pub type Stack = policy_stack![
    BudgetModule,
    E2eDeadlineGuardModule,
    EstimationModule,
    PredAdmissionModule
];

/// The hooks every generated client and server stub runs.
pub type Hooks = masa_policy::PolicyHooks<Stack>;
