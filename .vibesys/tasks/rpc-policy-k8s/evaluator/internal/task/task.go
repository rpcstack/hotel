// Package task wires the task on vibesys-evaluator's microservices runner: the application on
// Kubernetes, the benchmark, the accuracy check and the edit-scope guard (cmd/rpc-policy-k8s).
package task

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/uw-syfi/vibesys/sdk/vibesys-evaluator/go/check"
	"github.com/uw-syfi/vibesys/sdk/vibesys-evaluator/go/evaluator"
	"github.com/uw-syfi/vibesys/sdk/vibesys-evaluator/go/k8s"
	"github.com/uw-syfi/vibesys/sdk/vibesys-evaluator/go/microservices"
	"github.com/uw-syfi/vibesys/sdk/vibesys-evaluator/go/workload"

	"rpc-policy-k8s/evaluator/internal/hotel"
	"rpc-policy-k8s/evaluator/internal/oracle"
	"rpc-policy-k8s/evaluator/internal/scope"
	wl "rpc-policy-k8s/evaluator/internal/workload"
)

// The benchmark: capacity in a closed loop, then the p99 latency at a fixed offered rate in an
// open loop, the shape of the DeathStarBench and TrainTicket tasks. The numbers come from the
// base's runs on vibesys-k8s (README.md, "Choosing the loads").
const (
	ClosedClients = 128
	ClosedTrials  = 2
	ClosedSecs    = 60 * time.Second
	ClosedWarmup  = 15 * time.Second

	OpenRate   = 600.0
	OpenTrials = 3
	OpenSecs   = 60 * time.Second
	OpenWarmup = 10 * time.Second
)

// The accuracy check: AccuracyPhases phases of AccuracySecs at AccuracyRate, with faults between
// them.
const (
	AccuracyRate = 30.0
	AccuracySecs = 60 * time.Second
)

// Gateway is the frontend's HTTP API, the base URL of every request.
const Gateway = "http://frontend:5000"

// TargetEnv names the environment variable that runs the benchmark or the accuracy check
// against an application already running at its URL (docker compose; README.md, "Run it
// locally") instead of building and deploying the candidate on Kubernetes. The accuracy check
// then injects no faults, and the application must be fresh (nothing booked yet).
const TargetEnv = "HOTEL_TARGET"

func system() microservices.System {
	if target := os.Getenv(TargetEnv); target != "" {
		return microservices.Local(target)
	}
	return microservices.K8s(App(), Ready()...)
}

// Metrics are the benchmark's objectives.
var Metrics = map[string]evaluator.Metric{
	"operations_per_second": {Unit: "ops/s", Direction: evaluator.Max},
	"latency_ms.p99":        {Unit: "ms", Direction: evaluator.Min},
}

// Validity: no error in any window (a policy may not shed load by failing requests), every
// operation of the mix completed, the open loop offering at least 95% of its rate.
var Validity = evaluator.Validity{MaxErrorRate: 0, MinSustainedRatio: 0.95, MinOpsPerOperation: 1}

// App is the hotel on Kubernetes: the repository's manifests (k8s/base, rendered into
// k8s/hotel.yaml by k8s/render.sh) with one image built from the candidate's root Dockerfile.
func App() k8s.App {
	return k8s.App{
		Name:     "rpc-policy-k8s",
		Manifest: "k8s/hotel.yaml",
		Images:   map[string]string{"HOTEL_IMAGE": "."},
		Gateway:  Gateway,
		// The services: stateless processes (the user service reloads its accounts on start,
		// the profile and rate services their caches' sources). The databases are not
		// restartable: their data lives in an emptyDir, which a pod deletion loses.
		Restartable: []string{"frontend", "search", "geo", "rate", "profile", "recommendation", "reservation", "user"},
		Caches: map[string]k8s.CacheKind{
			"reservation-redis": k8s.Redis, "profile-redis": k8s.Redis, "rate-redis": k8s.Redis,
		},
	}
}

// Ready are the requests that must succeed before a sample's phases start: a login, a search and
// a recommendation, on a night no workload books.
func Ready() []workload.Step {
	return []workload.Step{
		workload.Get("/user", workload.Query("username", hotel.Username(0)), workload.Query("password", hotel.Password(0))),
		workload.Get("/hotels", workload.Query("inDate", "2015-03-01"), workload.Query("outDate", "2015-03-02"),
			workload.Query("lat", "38.0235"), workload.Query("lon", "-122.0950")),
		workload.Get("/recommendations", workload.Query("require", "rate"),
			workload.Query("lat", "38.0235"), workload.Query("lon", "-122.0950")),
	}
}

// Dataset is what the services seed themselves with at startup: the services write it, not a
// seed step, and the oracle checks every answer against it.
func Dataset() (*hotel.Dataset, error) { return hotel.Seeded(), nil }

// Task is the task: the benchmark (the DeathStarBench mix) and the accuracy check (its own
// workload, faults and oracle).
func Task() microservices.Task[*hotel.Dataset, *wl.AccModel] {
	bench := wl.Bench[*wl.AccModel]()
	acc := wl.Accuracy()
	faults := Faults()
	if os.Getenv(TargetEnv) != "" {
		faults = nil
	}
	return microservices.Task[*hotel.Dataset, *wl.AccModel]{
		Metrics:   Metrics,
		Validity:  &Validity,
		Dataset:   Dataset,
		System:    system(),
		Workload:  &bench,
		Benchmark: runBenchmark,
		Accuracy: &microservices.Accuracy[*hotel.Dataset, *wl.AccModel]{
			Phases:   wl.AccuracyPhases,
			Load:     microservices.Open{Rate: AccuracyRate, Secs: AccuracySecs, Arrivals: microservices.Uniform},
			Faults:   faults,
			Oracle:   oracle.Specs,
			Workload: &acc,
		},
	}
}

func runBenchmark(ctx context.Context, run *microservices.Run) error {
	var closed []float64
	for range ClosedTrials {
		r, err := run.ClosedLoop(ctx, microservices.Closed{Clients: ClosedClients, Secs: ClosedSecs, Warmup: ClosedWarmup})
		if err != nil {
			return err
		}
		closed = append(closed, r.OpsPerSec)
	}
	var p99 []float64
	for range OpenTrials {
		r, err := run.OpenLoop(ctx, microservices.Open{Rate: OpenRate, Secs: OpenSecs, Warmup: OpenWarmup, Arrivals: microservices.Uniform})
		if err != nil {
			return err
		}
		p99 = append(p99, r.P99Ms)
	}
	return run.Report(map[string]float64{"operations_per_second": median(closed), "latency_ms.p99": median(p99)})
}

func median(v []float64) float64 {
	s := append([]float64(nil), v...)
	sort.Float64s(s)
	if n := len(s); n%2 == 1 {
		return s[n/2]
	} else {
		return (s[n/2-1] + s[n/2]) / 2
	}
}

// Faults are the accuracy run's faults between its phases: the reservation service restarted
// (its Redis connections and nothing else), the three caches emptied in place, then the
// reservation cache's pod, the frontend and the user service (which reloads its accounts)
// restarted.
func Faults() []microservices.Fault {
	return []microservices.Fault{
		microservices.Restart(0, "reservation"),
		microservices.Flush(1, "reservation-redis"),
		microservices.Flush(1, "profile-redis"),
		microservices.Flush(1, "rate-redis"),
		microservices.Restart(2, "reservation-redis"),
		microservices.Restart(2, "frontend"),
		microservices.Restart(2, "user"),
	}
}

// ------------------------------------------------------------------ the scope guard

// Guard checks the candidate's edit scope before a `benchmark` or `accuracy` command builds
// anything (other commands, and an accuracy check of a captured history, pass through). Outside
// the scope, the benchmark gets an error record and the accuracy check FAIL (exit 1).
func Guard() {
	if len(os.Args) < 2 || (os.Args[1] != "benchmark" && os.Args[1] != "accuracy") {
		return
	}
	root := "."
	for i, a := range os.Args[2:] {
		switch {
		case a == "--history" || strings.HasPrefix(a, "--history="):
			return
		case a == "--project-root" && i+3 < len(os.Args):
			root = os.Args[i+3]
		case strings.HasPrefix(a, "--project-root="):
			root = strings.TrimPrefix(a, "--project-root=")
		}
	}
	problems := checkScope(root)
	if len(problems) == 0 {
		return
	}
	msg := "the candidate edits outside policy/ (the task's edit scope; OBJECTIVE.md): " + strings.Join(problems, "; ")
	if os.Args[1] == "accuracy" {
		fmt.Fprintln(os.Stderr, "FAIL:", msg)
		os.Exit(check.Fail.ExitCode())
	}
	rest := []string{os.Args[0], "benchmark"}
	for i := 2; i < len(os.Args); i++ {
		if os.Args[i] == "--project-root" {
			i++
			continue
		}
		if !strings.HasPrefix(os.Args[i], "--project-root=") {
			rest = append(rest, os.Args[i])
		}
	}
	os.Args = rest
	evaluator.Main(evaluator.Task{
		Metrics:   Metrics,
		Benchmark: func(context.Context, *evaluator.Run) error { return &evaluator.SampleError{Msg: msg} },
	})
}

func checkScope(root string) []string {
	exe, err := os.Executable()
	if err != nil {
		return []string{fmt.Sprintf("the task cannot find its scope manifest: %v", err)}
	}
	pinned, err := scope.Load(filepath.Join(filepath.Dir(exe), scope.File))
	if err != nil {
		return []string{fmt.Sprintf("the task's scope manifest: %v (a task bug)", err)}
	}
	return scope.Check(root, pinned)
}
