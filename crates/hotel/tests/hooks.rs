//! Regression test: every generated stub must default to the mode's hooks.
//!
//! rpcstack's tonic-build defaults generated stubs to
//! `tonic::rpcstack::noop::NoopHooks`. If `build.rs` forgets
//! `.default_hooks_path(...)` for any proto, that service runs without the
//! policy and nothing else fails. These checks compare the stubs' default hook
//! type parameter against the mode's hooks: `masa::DefaultHooks` in `baseline`
//! and `masa` mode, `hotel_policy::Hooks` (the `policy/` directory's stack) in
//! `policy` mode; and, in the modes that have a policy, that those hooks are
//! not the no-op hooks. (`build.rs` also fails the build on a stub that names
//! `NoopHooks`.)

use std::any::TypeId;

#[allow(unused_imports)]
use hotel::proto::*;
use tonic::rpcstack::noop::NoopHooks;
use tonic::transport::Channel;

/// The hooks the stubs of this build must default to.
#[cfg(feature = "policy")]
type ModeHooks = hotel_policy::Hooks;
#[cfg(not(feature = "policy"))]
type ModeHooks = masa::DefaultHooks;
#[cfg(feature = "policy")]
const MODE_HOOKS_PATH: &str = "hotel_policy::Hooks";
#[cfg(not(feature = "policy"))]
const MODE_HOOKS_PATH: &str = "masa::DefaultHooks";

const PROTOS: &[&str] = &[
    "frontend",
    "search",
    "geo",
    "rate",
    "profile",
    "reservation",
    "user",
    "recommendation",
    "review",
];

/// Type-level check for clients: `XClient<Channel>` (default hook parameter)
/// must be the same type as `XClient<Channel, ModeHooks>`.
macro_rules! assert_client_default {
    ($m:ident :: $client_mod:ident :: $client:ident) => {
        assert_eq!(
            TypeId::of::<$m::$client_mod::$client<Channel>>(),
            TypeId::of::<$m::$client_mod::$client<Channel, ModeHooks>>(),
            "{} does not default to {}",
            stringify!($client),
            MODE_HOOKS_PATH
        );
    };
}

#[test]
fn generated_clients_default_to_mode_hooks() {
    assert_client_default!(frontend::frontend_client::FrontendClient);
    assert_client_default!(search::search_client::SearchClient);
    assert_client_default!(geo::geo_client::GeoClient);
    assert_client_default!(rate::rate_client::RateClient);
    assert_client_default!(profile::profile_client::ProfileClient);
    assert_client_default!(reservation::reservation_client::ReservationClient);
    assert_client_default!(user::user_client::UserClient);
    assert_client_default!(recommendation::recommendation_client::RecommendationClient);
    assert_client_default!(review::review_client::ReviewClient);
}

/// Source-level check covering servers too (their type needs a service impl,
/// which lives in the binaries): every default hook parameter in the generated
/// code must be the mode's hooks, one for each client and each server.
#[test]
fn generated_sources_default_to_mode_hooks() {
    for proto in PROTOS {
        let path = std::path::Path::new(env!("OUT_DIR")).join(format!("{proto}.rs"));
        let src = std::fs::read_to_string(&path).expect("generated source");
        assert!(
            !src.contains("NoopHooks"),
            "{proto}: generated code names NoopHooks"
        );
        let defaults: Vec<&str> = src
            .split("tonic::rpcstack::Hooks =")
            .skip(1)
            .map(|rest| {
                rest.trim_start()
                    .split(|c: char| c == '>' || c == ',')
                    .next()
                    .unwrap()
                    .trim()
            })
            .collect();
        assert_eq!(
            defaults.len(),
            2,
            "{proto}: expected a client and a server default, got {defaults:?}"
        );
        for d in defaults {
            assert_eq!(d, MODE_HOOKS_PATH, "{proto}: default hooks are {d}");
        }
    }
}

/// In Masa mode the generated stubs must actually run the policy.
#[cfg(feature = "masa")]
#[test]
fn masa_mode_does_not_fall_back_to_noop_hooks() {
    assert_ne!(
        TypeId::of::<masa::DefaultHooks>(),
        TypeId::of::<NoopHooks>(),
        "masa feature is on but masa::DefaultHooks resolved to NoopHooks"
    );
    assert_ne!(
        TypeId::of::<frontend::frontend_client::FrontendClient<Channel>>(),
        TypeId::of::<frontend::frontend_client::FrontendClient<Channel, NoopHooks>>(),
    );
}

/// In policy mode the stubs run the policy directory's stack, which is neither
/// the no-op hooks nor (necessarily) Masa's default; every client must carry it.
#[cfg(feature = "policy")]
#[test]
fn policy_mode_stubs_run_the_policy_directory_stack() {
    assert_ne!(
        TypeId::of::<hotel_policy::Hooks>(),
        TypeId::of::<NoopHooks>(),
        "policy mode but hotel_policy::Hooks is the no-op hooks"
    );
    assert_ne!(
        TypeId::of::<frontend::frontend_client::FrontendClient<Channel>>(),
        TypeId::of::<frontend::frontend_client::FrontendClient<Channel, NoopHooks>>(),
    );
    // The stack must be the declared one, not a stand-in type with the same name.
    assert_eq!(
        TypeId::of::<ModeHooks>(),
        TypeId::of::<masa_policy::PolicyHooks<hotel_policy::Stack>>()
    );
}

/// In policy mode the run queue Tokio uses is the policy directory's
/// `queue.rs`, not the framework's `custom.rs` or FIFO/priority default.
#[cfg(feature = "policy")]
#[test]
fn policy_mode_run_queue_is_the_policy_queue() {
    assert_eq!(
        rpcstack_sched::external::POLICY_QUEUE_ID,
        hotel_policy::queue::POLICY_QUEUE_ID
    );
    assert_eq!(
        TypeId::of::<rpcstack_sched::SelectedQueue<u64>>(),
        TypeId::of::<rpcstack_sched::external::Queue<u64>>(),
    );
}

/// Baseline mode is the no-op hook stack: stubs pay no policy cost.
#[cfg(not(any(
    feature = "policy",
    feature = "sched_fifo",
    feature = "sched_slo",
    feature = "sched_tailclipper",
    feature = "sched_oracle"
)))]
#[test]
fn baseline_mode_uses_noop_hooks() {
    assert_eq!(
        TypeId::of::<masa::DefaultHooks>(),
        TypeId::of::<NoopHooks>()
    );
}
