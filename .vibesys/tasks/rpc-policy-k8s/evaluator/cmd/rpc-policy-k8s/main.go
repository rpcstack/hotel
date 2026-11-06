// Command rpc-policy-k8s is the task: `benchmark --project-root DIR` (one sample) and
// `accuracy --project-root DIR` (the accuracy check: exit 0 PASS, 1 FAIL, 2 UNKNOWN; with
// `--history DIR` it judges a captured history again), and the commands vibesys-evaluator runs
// in its pods (`phase`, `generate`, `accuracy-workload generate`, `seed`).
package main

import (
	"github.com/uw-syfi/vibesys/sdk/vibesys-evaluator/go/microservices"

	"rpc-policy-k8s/evaluator/internal/task"
)

func main() {
	task.Guard()
	microservices.Main(task.Task())
}
