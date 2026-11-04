//! Regression test: every generated stub must default to `masa::DefaultHooks`.
//!
//! rpcstack's tonic-build defaults generated stubs to
//! `tonic::rpcstack::noop::NoopHooks`. If `build.rs` forgets
//! `.default_hooks_path("masa::DefaultHooks")` for any proto, that service runs
//! without the policy and nothing else fails. These checks compare the stubs'
//! default hook type parameter against `masa::DefaultHooks`, and in Masa mode
//! also check that `masa::DefaultHooks` is not the no-op hooks.

use std::any::TypeId;

#[allow(unused_imports)]
use hotel::proto::*;
use tonic::rpcstack::noop::NoopHooks;
use tonic::transport::Channel;

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
/// must be the same type as `XClient<Channel, masa::DefaultHooks>`.
macro_rules! assert_client_default {
    ($m:ident :: $client_mod:ident :: $client:ident) => {
        assert_eq!(
            TypeId::of::<$m::$client_mod::$client<Channel>>(),
            TypeId::of::<$m::$client_mod::$client<Channel, masa::DefaultHooks>>(),
            "{} does not default to masa::DefaultHooks",
            stringify!($client)
        );
    };
}

#[test]
fn generated_clients_default_to_masa_hooks() {
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
/// code must be `masa::DefaultHooks`, one for each client and each server.
#[test]
fn generated_sources_default_to_masa_hooks() {
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
            assert_eq!(d, "masa::DefaultHooks", "{proto}: default hooks are {d}");
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

/// Baseline mode is the no-op hook stack: stubs pay no policy cost.
#[cfg(not(any(
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
