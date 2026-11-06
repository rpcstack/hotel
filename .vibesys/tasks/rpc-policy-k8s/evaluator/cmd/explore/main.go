// Command explore is the benchmark with other loads, for choosing the benchmark's (README.md,
// "Choosing the loads"): one sample that runs a closed loop for each client count of
// EXPLORE_CLIENTS, then an open loop for each rate of EXPLORE_RATES (comma-separated), with
// EXPLORE_SECS-second windows, and logs every phase's throughput and latency. Errors do not end
// it. It is not part of the task's commands.
//
//	EXPLORE_CLIENTS=64,128,256 EXPLORE_RATES=500,1000 explore benchmark --project-root DIR --vs-output FILE
package main

import (
	"context"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/uw-syfi/vibesys/sdk/vibesys-evaluator/go/evaluator"
	"github.com/uw-syfi/vibesys/sdk/vibesys-evaluator/go/microservices"

	"rpc-policy-k8s/evaluator/internal/task"
)

func list(name string) []float64 {
	var out []float64
	for _, f := range strings.Split(os.Getenv(name), ",") {
		if v, err := strconv.ParseFloat(strings.TrimSpace(f), 64); err == nil {
			out = append(out, v)
		}
	}
	return out
}

func main() {
	t := task.Task()
	t.Accuracy = nil
	t.Validity = &evaluator.Validity{MaxErrorRate: 1, MinSustainedRatio: 0.01, MinOpsPerOperation: 0}
	secs := 30 * time.Second
	if s, err := strconv.Atoi(os.Getenv("EXPLORE_SECS")); err == nil && s > 0 {
		secs = time.Duration(s) * time.Second
	}
	t.Benchmark = func(ctx context.Context, run *microservices.Run) error {
		var ops, p99 float64
		for _, c := range list("EXPLORE_CLIENTS") {
			r, err := run.ClosedLoop(ctx, microservices.Closed{Clients: int(c), Secs: secs, Warmup: 10 * time.Second})
			if err != nil {
				evaluator.Log("explore: closed %v clients: %v", c, err)
				continue
			}
			evaluator.Log("explore: closed %v clients: %.1f ops/s, p50 %.1f ms, p99 %.1f ms", c, r.OpsPerSec, r.P50Ms, r.P99Ms)
			ops = max(ops, r.OpsPerSec)
		}
		for _, rate := range list("EXPLORE_RATES") {
			r, err := run.OpenLoop(ctx, microservices.Open{Rate: rate, Secs: secs, Warmup: 10 * time.Second, Arrivals: microservices.Uniform})
			if err != nil {
				evaluator.Log("explore: open %v/s: %v", rate, err)
				continue
			}
			evaluator.Log("explore: open %v/s: %.1f ops/s, p50 %.1f ms, p99 %.1f ms", rate, r.OpsPerSec, r.P50Ms, r.P99Ms)
			p99 = r.P99Ms
		}
		return run.Report(map[string]float64{"operations_per_second": ops, "latency_ms.p99": p99})
	}
	microservices.Main(t)
}
