// Command scope writes or checks the task's edit-scope manifest (scope.json):
//
//	scope write --project-root DIR [--out FILE]    pin DIR's protected files and Cargo.lock
//	scope check --project-root DIR [--manifest FILE]
//
// Regenerate the manifest whenever the task moves to another commit of the application.
package main

import (
	"flag"
	"fmt"
	"os"

	"rpc-policy-k8s/evaluator/internal/scope"
)

func main() {
	if len(os.Args) < 2 || (os.Args[1] != "write" && os.Args[1] != "check") {
		fmt.Fprintln(os.Stderr, "usage: scope write|check --project-root DIR [--out FILE | --manifest FILE]")
		os.Exit(2)
	}
	set := flag.NewFlagSet(os.Args[1], flag.ExitOnError)
	root := set.String("project-root", ".", "the project's checkout")
	file := set.String("out", scope.File, "the manifest to write")
	manifest := set.String("manifest", scope.File, "the manifest to check against")
	_ = set.Parse(os.Args[2:])
	if os.Args[1] == "write" {
		if err := scope.Write(*root, *file); err != nil {
			fmt.Fprintln(os.Stderr, "scope:", err)
			os.Exit(1)
		}
		return
	}
	pinned, err := scope.Load(*manifest)
	if err != nil {
		fmt.Fprintln(os.Stderr, "scope:", err)
		os.Exit(2)
	}
	if problems := scope.Check(*root, pinned); len(problems) > 0 {
		for _, p := range problems {
			fmt.Println(p)
		}
		os.Exit(1)
	}
	fmt.Println("in scope")
}
