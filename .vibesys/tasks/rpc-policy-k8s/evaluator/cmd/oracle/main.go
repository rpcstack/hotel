// Command oracle judges a captured accuracy history with the task's oracle, offline:
//
//	oracle --history DIR [--out DIR] [budget flags]   (check.Main; exit 0 PASS, 1 FAIL, 2 UNKNOWN)
//	oracle stats DIR                                  what the history exercised
package main

import (
	"fmt"
	"os"

	"github.com/uw-syfi/vibesys/sdk/vibesys-evaluator/go/check"

	"rpc-policy-k8s/evaluator/internal/oracle"
)

func main() {
	if len(os.Args) == 3 && os.Args[1] == "stats" {
		h, err := check.OpenHistory(os.Args[2])
		if err == nil {
			err = oracle.Stats(h, os.Stdout)
		}
		if err != nil {
			fmt.Fprintln(os.Stderr, "oracle stats:", err)
			os.Exit(2)
		}
		return
	}
	check.Main(oracle.Specs)
}
