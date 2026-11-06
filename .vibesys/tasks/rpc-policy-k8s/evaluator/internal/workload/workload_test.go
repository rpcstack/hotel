package workload

import (
	"encoding/json"
	"testing"

	"github.com/uw-syfi/vibesys/sdk/vibesys-evaluator/go/workload"

	"rpc-policy-k8s/evaluator/internal/hotel"
)

func TestPlans(t *testing.T) {
	data := hotel.Seeded()
	m, err := NewAccModel(data, workload.NewRand(7, "model"))
	if err != nil {
		t.Fatal(err)
	}
	seen := map[string]bool{}
	for p := 0; p < AccuracyPhases; p++ {
		plans := m.Plans(p)
		if want := NightsPerPhase * (len(data.Rated()) + unratedPerNight); len(plans) != want {
			t.Fatalf("phase %d: %d plans, want %d", p, len(plans), want)
		}
		for _, pl := range plans {
			key := hotel.Key(pl.Hotel, pl.Night)
			if seen[key] {
				t.Fatalf("%s planned twice", key)
			}
			seen[key] = true
			if len(pl.Rooms) != steps {
				t.Fatalf("%s: %v", key, pl.Rooms)
			}
			free := data.Hotel(pl.Hotel).Capacity - data.BookedRooms(pl.Hotel, pl.Night)
			booked := 0
			var outcomes []bool
			for _, n := range pl.Rooms {
				if n < 1 {
					t.Fatalf("%s %s: %v", key, pl.Kind, pl.Rooms)
				}
				ok := booked+n <= free
				if ok {
					booked += n
				}
				outcomes = append(outcomes, ok)
			}
			want := map[string][]bool{
				PlanExact: {true, true, true, false}, PlanPartial: {true, true, false, true},
				PlanOver: {true, false, true, false}, PlanLight: {true, true, true, true},
			}[pl.Kind]
			for i := range want {
				if outcomes[i] != want[i] {
					t.Fatalf("%s %s %v: outcomes %v, want %v", key, pl.Kind, pl.Rooms, outcomes, want)
				}
			}
			if pl.Kind != PlanLight && booked != free {
				t.Fatalf("%s %s %v: ends with %d of %d free rooms booked", key, pl.Kind, pl.Rooms, booked, free)
			}
		}
	}
	if !seen[hotel.Key("4", "2015-04-09")] {
		t.Fatal("the seeded night is not planned")
	}
}

// stream builds a phase's operations the way the generator does: a weighted draw of the mix,
// then the operation's Build on its own stream.
func stream(t *testing.T, phase, n int) []hotel.Info {
	t.Helper()
	w := Accuracy()
	m, err := w.Model(hotel.Seeded(), workload.NewRand(7, "model"))
	if err != nil {
		t.Fatal(err)
	}
	if err := w.BeginPhase(m, phase); err != nil {
		t.Fatal(err)
	}
	weights := make([]float64, len(w.Operations))
	for i, op := range w.Operations {
		weights[i] = op.Weight
	}
	mix := workload.NewRand(7, "mix", phase)
	var infos []hotel.Info
	for i := 0; i < n; i++ {
		op := w.Operations[mix.Weighted(weights)]
		req := op.Build(m, workload.NewRand(7, "op", phase, i))
		info := req.Info.(hotel.Info)
		if info.Kind != op.Name {
			t.Fatalf("operation %s has info %+v", op.Name, info)
		}
		if _, err := json.Marshal(info); err != nil {
			t.Fatal(err)
		}
		infos = append(infos, info)
	}
	return infos
}

func TestStreamKeepsReservationsApart(t *testing.T) {
	ops := int(30 * 60) // a phase at the accuracy rate
	for phase := 0; phase < AccuracyPhases; phase++ {
		infos := stream(t, phase, ops)
		last := map[string]int{}
		minGap, searched := ops, map[string]bool{}
		for i, info := range infos {
			if i < PrefixOps {
				if info.Kind == "reserve" && info.Valid {
					t.Fatalf("phase %d: operation %d reserves in the prefix", phase, i)
				}
				if info.Kind == "search" {
					searched[info.Night] = true
				}
			}
			if info.Kind != "reserve" || !info.Valid {
				continue
			}
			key := hotel.Key(info.Hotel, info.Night)
			if j, ok := last[key]; ok {
				minGap = min(minGap, i-j)
			}
			last[key] = i
		}
		// Every night booked so far is searched in the prefix, so the cache holds it.
		for n := 0; n < (phase+1)*NightsPerPhase; n++ {
			if !searched[Night(n)] {
				t.Errorf("phase %d: night %s is not searched in the prefix", phase, Night(n))
			}
		}
		// At 30 ops/s, 150 operations are 5 s: far longer than a reservation takes.
		if minGap < 150 {
			t.Errorf("phase %d: two reservations of one night %d operations apart", phase, minGap)
		}
		t.Logf("phase %d: reservations of one night at least %d operations apart", phase, minGap)
	}
}

func TestRecommendationsAvoidTies(t *testing.T) {
	infos := stream(t, 0, 3000)
	data := hotel.Seeded()
	n := 0
	for _, info := range infos {
		if info.Kind != "recommend" || info.Require != "dis" {
			continue
		}
		n++
		var lat, lon float64
		_ = json.Unmarshal([]byte(info.Lat), &lat)
		_ = json.Unmarshal([]byte(info.Lon), &lon)
		if allowed, ambiguous := data.Recommend("dis", lat, lon); ambiguous || len(allowed) != 1 {
			t.Errorf("a recommendation at a tie: %+v %v", info, allowed)
		}
	}
	if n == 0 {
		t.Fatal("no recommendation by distance")
	}
}
