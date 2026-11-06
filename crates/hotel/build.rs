use std::{env, path::PathBuf};

/// Every generated client and server must default to the mode's hooks. Without
/// this, rpcstack's tonic-build falls back to `tonic::rpcstack::noop::NoopHooks`
/// and a policy silently does nothing. `tests/hooks.rs` guards this too.
///
/// `baseline` uses the framework's no-op hooks (spelled out, so the default is
/// explicit); `policy` uses the stack the policy directory defines
/// (`policy/src/stack.rs`), and there a stub that names `NoopHooks` anywhere
/// fails the build.
const NOOP_HOOKS: &str = "tonic::rpcstack::noop::NoopHooks";

fn policy_mode() -> bool {
    env::var_os("CARGO_FEATURE_POLICY").is_some()
}

fn default_hooks() -> &'static str {
    if policy_mode() {
        "hotel_policy::Hooks"
    } else {
        NOOP_HOOKS
    }
}

/// Fail the build if a generated stub does not default to `hooks` (client and
/// server: exactly two defaults), or, in policy mode, names the no-op hooks.
fn assert_generated_hooks(out_dir: &std::path::Path, name: &str, hooks: &str) {
    let src = std::fs::read_to_string(out_dir.join(format!("{name}.rs")))
        .unwrap_or_else(|e| panic!("generated {name}.rs: {e}"));
    if policy_mode() {
        assert!(
            !src.contains("NoopHooks"),
            "{name}: generated code names NoopHooks in policy mode; set default_hooks_path"
        );
    }
    let n = src
        .matches(&format!("tonic::rpcstack::Hooks = {hooks}"))
        .count();
    assert_eq!(
        n, 2,
        "{name}: expected client and server to default to {hooks}, found {n}"
    );
}

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

fn main() {
    // Vendored protoc: no system protobuf-compiler needed.
    std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path().unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let hooks = default_hooks();
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_POLICY");
    for name in PROTOS {
        let mut builder = tonic_build::configure()
            .default_hooks_path(hooks)
            .file_descriptor_set_path(out_dir.join(format!("{name}_descriptor.bin")));
        if *name == "review" {
            builder =
                builder.type_attribute(".", "#[derive(serde::Deserialize, serde::Serialize)]");
        }
        let proto = format!("proto/{name}.proto");
        println!("cargo:rerun-if-changed={proto}");
        builder.compile(&[proto], &["proto"]).unwrap();
        assert_generated_hooks(&out_dir, name, hooks);
    }
}
