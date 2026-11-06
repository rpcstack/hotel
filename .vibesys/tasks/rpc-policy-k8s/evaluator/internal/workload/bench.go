// Package workload holds the task's two request generators on the vibesys-evaluator Go SDK:
// Bench, the benchmark's mix, and Accuracy, the accuracy check's.
package workload

import (
	"fmt"
	"strconv"

	"github.com/uw-syfi/vibesys/sdk/vibesys-evaluator/go/workload"

	"rpc-policy-k8s/evaluator/internal/hotel"
)

// The benchmark's mix is DeathStarBench's hotelReservation wrk2 script
// (wrk2/scripts/hotel-reservation/mixed-workload_type_1.lua): 60% searches, 39% recommendations,
// 0.5% logins and 0.5% reservations, every request drawn the way that script draws it.
const (
	SearchWeight    = 600
	RecommendWeight = 390
	LoginWeight     = 5
	ReserveWeight   = 5
	// wrk2 draws users Cornell_0 .. Cornell_500 (math.random(0, 500)); here Username_0 .. 500.
	benchUsers = 501
)

// location is the script's: lat 38.0235 + (random(0, 481) - 240.5) / 1000, lon -122.095 +
// (random(0, 325) - 157) / 1000.
func location(rand *workload.Rand) (lat, lon float64) {
	lat = 38.0235 + (float64(rand.Below(482))-240.5)/1000.0
	lon = -122.095 + (float64(rand.Below(326))-157.0)/1000.0
	return lat, lon
}

func coord(v float64) string { return strconv.FormatFloat(v, 'f', 4, 64) }

// april is 2015-04-<day>, the script's dates.
func april(day int) string { return fmt.Sprintf("2015-04-%02d", day) }

// requireOf is the script's coin: dis below 0.33, rate below 0.66, price otherwise.
func requireOf(rand *workload.Rand) string {
	switch coin := rand.Random(); {
	case coin < 0.33:
		return "dis"
	case coin < 0.66:
		return "rate"
	default:
		return "price"
	}
}

// Bench is the benchmark's workload: no model (M is the accuracy workload's model type, which a
// task's two workloads share; the benchmark's is M's zero value) and no info (a benchmark only
// measures).
func Bench[M any]() workload.Workload[*hotel.Dataset, M] {
	search := func(_ M, rand *workload.Rand) workload.Request {
		in := 9 + rand.Below(15)          // random(9, 23)
		out := in + 1 + rand.Below(24-in) // random(in + 1, 24)
		lat, lon := location(rand)
		return workload.Request{Steps: []workload.Step{workload.Get("/hotels",
			workload.Query("inDate", april(in)), workload.Query("outDate", april(out)),
			workload.Query("lat", coord(lat)), workload.Query("lon", coord(lon)))}}
	}
	recommend := func(_ M, rand *workload.Rand) workload.Request {
		require := requireOf(rand)
		lat, lon := location(rand)
		return workload.Request{Steps: []workload.Step{workload.Get("/recommendations",
			workload.Query("require", require), workload.Query("lat", coord(lat)), workload.Query("lon", coord(lon)))}}
	}
	login := func(_ M, rand *workload.Rand) workload.Request {
		user := rand.Below(benchUsers)
		return workload.Request{Steps: []workload.Step{workload.Get("/user",
			workload.Query("username", hotel.Username(user)), workload.Query("password", hotel.Password(user)))}}
	}
	reserve := func(_ M, rand *workload.Rand) workload.Request {
		in := 9 + rand.Below(15)      // random(9, 23)
		out := in + 1 + rand.Below(5) // in + random(1, 5)
		hotelID := 1 + rand.Below(80) // random(1, 80)
		user := rand.Below(benchUsers)
		return workload.Request{Steps: []workload.Step{workload.Get("/reservation",
			workload.Query("inDate", april(in)), workload.Query("outDate", april(out)),
			workload.Query("hotelId", strconv.Itoa(hotelID)), workload.Query("customerName", hotel.Username(user)),
			workload.Query("username", hotel.Username(user)), workload.Query("password", hotel.Password(user)),
			workload.Query("number", "1"))}}
	}
	return workload.Workload[*hotel.Dataset, M]{
		Dataset: hotel.ReadDataset,
		Model:   func(*hotel.Dataset, *workload.Rand) (M, error) { var zero M; return zero, nil },
		Operations: []workload.Operation[M]{
			{Name: "search", Weight: SearchWeight, Build: search},
			{Name: "recommend", Weight: RecommendWeight, Build: recommend},
			{Name: "login", Weight: LoginWeight, Build: login},
			{Name: "reserve", Weight: ReserveWeight, Build: reserve},
		},
	}
}
