package scope

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

const lock = `version = 4

[[package]]
name = "hotel-policy"
version = "0.1.0"
dependencies = [
 "tokio",
]

[[package]]
name = "tokio"
version = "1.0.0"
source = "git+https://example/rpcstack"
`

func project(t *testing.T) string {
	t.Helper()
	root := t.TempDir()
	for path, content := range map[string]string{
		"Dockerfile": "FROM x\n", "Cargo.toml": "[workspace]\n", "Cargo.lock": lock,
		"crates/hotel/src/lib.rs": "fn main() {}\n", "policy/Cargo.toml": "[package]\nname = \"hotel-policy\"\n",
		"policy/src/lib.rs": "\n", "README.md": "readme\n",
	} {
		p := filepath.Join(root, path)
		if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(p, []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	return root
}

func write(t *testing.T, root, path, content string) {
	t.Helper()
	p := filepath.Join(root, path)
	_ = os.MkdirAll(filepath.Dir(p), 0o755)
	if err := os.WriteFile(p, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

func TestScope(t *testing.T) {
	root := project(t)
	pinned, err := Read(root)
	if err != nil {
		t.Fatal(err)
	}
	if p := Check(root, pinned); len(p) != 0 {
		t.Fatalf("pristine: %v", p)
	}
	// In scope: policy/, files the image does not read, a crate added for the policy.
	write(t, root, "policy/src/lib.rs", "pub mod x;\n")
	write(t, root, "README.md", "changed\n")
	write(t, root, "Cargo.lock", strings.Replace(lock, " \"tokio\",\n", " \"serde\",\n \"tokio\",\n", 1)+
		"\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"ab\"\n")
	if p := Check(root, pinned); len(p) != 0 {
		t.Fatalf("in scope: %v", p)
	}
	// Out of scope.
	write(t, root, "crates/hotel/src/lib.rs", "fn main() { evil() }\n")
	write(t, root, "crates/hotel/src/new.rs", "\n")
	write(t, root, "Cargo.lock", strings.Replace(lock, "1.0.0", "1.0.1", 1))
	write(t, root, "policy/build.rs", "fn main() {}\n")
	got := strings.Join(Check(root, pinned), "\n")
	for _, want := range []string{"crates/hotel/src/lib.rs was changed", "crates/hotel/src/new.rs was added",
		"no longer has tokio 1.0.0", "build script"} {
		if !strings.Contains(got, want) {
			t.Errorf("missing %q in:\n%s", want, got)
		}
	}
}
