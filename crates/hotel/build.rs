use std::{env, path::PathBuf};

/// Every generated client and server must default to the mode's hooks. Without
/// this, rpcstack's tonic-build falls back to `tonic::rpcstack::noop::NoopHooks`
/// and the policy silently does nothing. `tests/hooks.rs` guards this too.
///
/// `baseline` and `masa` use Masa's feature-selected `masa::DefaultHooks`; the
/// `policy` mode uses the stack the policy directory defines
/// (`policy/src/stack.rs`).
fn default_hooks() -> &'static str {
    if env::var_os("CARGO_FEATURE_POLICY").is_some() {
        "hotel_policy::Hooks"
    } else {
        "masa::DefaultHooks"
    }
}

/// Fail the build if a generated stub does not default to `hooks` (client and
/// server: exactly two defaults) or names the no-op hooks anywhere.
fn assert_generated_hooks(out_dir: &std::path::Path, name: &str, hooks: &str) {
    let src = std::fs::read_to_string(out_dir.join(format!("{name}.rs")))
        .unwrap_or_else(|e| panic!("generated {name}.rs: {e}"));
    assert!(
        !src.contains("NoopHooks"),
        "{name}: generated code names NoopHooks; set default_hooks_path"
    );
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
