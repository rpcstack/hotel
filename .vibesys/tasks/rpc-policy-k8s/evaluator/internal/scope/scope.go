// Package scope enforces the task's edit scope: a candidate may change policy/ (and add the
// crates its policy needs to Cargo.lock), nothing else that goes into the image the task builds.
//
// The task schema has no key for an editable scope, so the task binaries check it themselves,
// before they build anything, against a manifest pinned in the task directory (scope.json): the
// digest of every file outside policy/ that the image build reads (the Dockerfile copies them),
// and Cargo.lock's packages. A candidate outside the scope gets an error record from the benchmark
// and FAIL from the accuracy check.
package scope

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/BurntSushi/toml"
)

// Protected are the paths, files or directories, outside policy/ that the image build reads
// (Dockerfile: its COPY lines, and .dockerignore deciding the context). Directories are pinned
// whole: no file may change, appear or disappear.
var Protected = []string{
	".cargo", ".dockerignore", "Cargo.toml", "Dockerfile", "config", "crates", "docker", "rust-toolchain.toml",
}

// Policy is the package the candidate edits: its dependencies in Cargo.lock may change.
const Policy = "hotel-policy"

// File is the manifest's name in the task directory.
const File = "scope.json"

// Manifest pins the protected files and Cargo.lock's packages.
type Manifest struct {
	// Files maps each protected file (slash-separated, relative to the project root) to the
	// SHA-256 of its content.
	Files map[string]string `json:"files"`
	// LockVersion and Packages are Cargo.lock's.
	LockVersion int       `json:"lock_version"`
	Packages    []Package `json:"packages"`
}

// Package is a Cargo.lock package; Dependencies are names only (Cargo adds a version to a name
// only when the lock has two versions of it, which an added crate may cause).
type Package struct {
	Name         string   `json:"name"`
	Version      string   `json:"version"`
	Source       string   `json:"source,omitempty"`
	Checksum     string   `json:"checksum,omitempty"`
	Dependencies []string `json:"dependencies,omitempty"`
}

func (p Package) id() string { return p.Name + " " + p.Version + " " + p.Source }

// Read reads the project's protected files and Cargo.lock into a manifest.
func Read(root string) (*Manifest, error) {
	m := &Manifest{Files: map[string]string{}}
	for _, top := range Protected {
		err := filepath.WalkDir(filepath.Join(root, top), func(path string, d fs.DirEntry, err error) error {
			if err != nil {
				if errors.Is(err, fs.ErrNotExist) && path == filepath.Join(root, top) {
					return nil // a protected path that does not exist is pinned as absent
				}
				return err
			}
			if d.IsDir() {
				return nil
			}
			rel, err := filepath.Rel(root, path)
			if err != nil {
				return err
			}
			var content []byte
			if d.Type()&fs.ModeSymlink != 0 {
				target, err := os.Readlink(path)
				if err != nil {
					return err
				}
				content = []byte("symlink:" + target)
			} else if content, err = os.ReadFile(path); err != nil {
				return err
			}
			sum := sha256.Sum256(content)
			m.Files[filepath.ToSlash(rel)] = hex.EncodeToString(sum[:])
			return nil
		})
		if err != nil {
			return nil, err
		}
	}
	var lock struct {
		Version int `toml:"version"`
		Package []struct {
			Name         string   `toml:"name"`
			Version      string   `toml:"version"`
			Source       string   `toml:"source"`
			Checksum     string   `toml:"checksum"`
			Dependencies []string `toml:"dependencies"`
		} `toml:"package"`
	}
	if _, err := toml.DecodeFile(filepath.Join(root, "Cargo.lock"), &lock); err != nil {
		return nil, fmt.Errorf("Cargo.lock: %w", err)
	}
	m.LockVersion = lock.Version
	for _, p := range lock.Package {
		deps := make([]string, 0, len(p.Dependencies))
		for _, d := range p.Dependencies {
			name, _, _ := strings.Cut(d, " ")
			deps = append(deps, name)
		}
		sort.Strings(deps)
		m.Packages = append(m.Packages, Package{p.Name, p.Version, p.Source, p.Checksum, deps})
	}
	return m, nil
}

// Check compares the project at root with the pinned manifest, and checks that policy/ brings no
// build script: it returns every way the project leaves the scope, nil when it does not.
func Check(root string, pinned *Manifest) []string {
	got, err := Read(root)
	if err != nil {
		return []string{fmt.Sprintf("the project cannot be read: %v", err)}
	}
	var problems []string
	for path, sum := range pinned.Files {
		switch now, ok := got.Files[path]; {
		case !ok:
			problems = append(problems, path+" was removed")
		case now != sum:
			problems = append(problems, path+" was changed")
		}
	}
	for path := range got.Files {
		if _, ok := pinned.Files[path]; !ok {
			problems = append(problems, path+" was added")
		}
	}
	if got.LockVersion != pinned.LockVersion {
		problems = append(problems, fmt.Sprintf("Cargo.lock's format changed (version %d, was %d)", got.LockVersion, pinned.LockVersion))
	}
	have := map[string]Package{}
	for _, p := range got.Packages {
		have[p.id()] = p
	}
	for _, p := range pinned.Packages {
		now, ok := have[p.id()]
		switch {
		case !ok:
			problems = append(problems, fmt.Sprintf("Cargo.lock no longer has %s %s (only crates may be added)", p.Name, p.Version))
		case now.Checksum != p.Checksum:
			problems = append(problems, fmt.Sprintf("Cargo.lock changed the checksum of %s %s", p.Name, p.Version))
		case p.Name != Policy && strings.Join(now.Dependencies, ",") != strings.Join(p.Dependencies, ","):
			problems = append(problems, fmt.Sprintf("Cargo.lock changed the dependencies of %s %s", p.Name, p.Version))
		}
	}
	problems = append(problems, buildScript(root)...)
	sort.Strings(problems)
	return problems
}

// buildScript refuses a build script or procedural macro in policy/: code that runs at build time
// could change the application's sources before they compile.
func buildScript(root string) []string {
	var problems []string
	if _, err := os.Stat(filepath.Join(root, "policy", "build.rs")); err == nil {
		problems = append(problems, "policy/build.rs: the policy crate may not have a build script")
	}
	var manifest struct {
		Package map[string]any `toml:"package"`
		Lib     map[string]any `toml:"lib"`
	}
	if _, err := toml.DecodeFile(filepath.Join(root, "policy", "Cargo.toml"), &manifest); err != nil {
		return append(problems, fmt.Sprintf("policy/Cargo.toml: %v", err))
	}
	if b, ok := manifest.Package["build"]; ok && b != false {
		problems = append(problems, "policy/Cargo.toml: the policy crate may not have a build script (package.build)")
	}
	for _, key := range []string{"proc-macro", "proc_macro"} {
		if v, ok := manifest.Lib[key]; ok && v != false {
			problems = append(problems, "policy/Cargo.toml: the policy crate may not be a procedural macro")
		}
	}
	return problems
}

// Load reads a manifest file.
func Load(path string) (*Manifest, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var m Manifest
	if err := json.Unmarshal(raw, &m); err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	if len(m.Files) == 0 || len(m.Packages) == 0 {
		return nil, fmt.Errorf("%s pins no files or no packages", path)
	}
	return &m, nil
}

// Write writes the manifest of the project at root to path.
func Write(root, path string) error {
	m, err := Read(root)
	if err != nil {
		return err
	}
	if problems := buildScript(root); len(problems) > 0 {
		return errors.New(strings.Join(problems, "; "))
	}
	raw, err := json.MarshalIndent(m, "", " ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, append(raw, '\n'), 0o644)
}
