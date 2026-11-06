package workload

import (
	"fmt"
	"strconv"

	"github.com/uw-syfi/vibesys/sdk/vibesys-evaluator/go/workload"

	"rpc-policy-k8s/evaluator/internal/hotel"
)

// The accuracy workload. It runs at a fixed low open-loop rate (task.AccuracyRate) in
// AccuracyPhases phases, and is built so that the application as it stands (the no-op policy)
// answers every request the way the oracle's model says, while a policy that serves stale,
// fabricated, lost or duplicated answers does not:
//
//   - Every phase reserves rooms on nights of its own (NightsPerPhase nights, a block after the
//     earlier phases'): on each of the 27 hotels a search returns, and on a few others, a plan of
//     four reservations that takes the night exactly to full, to one room short of full and then
//     full, past full by one room (refused) and then full, or leaves it far from full. Searches of
//     those nights then show each hotel exactly while it has a room left.
//   - The reservation service checks and books a night with separate cache reads and writes, so
//     two concurrent reservations of one night may both succeed against the same count (the
//     application's own race, not a policy's). Reservations therefore never overlap on a night:
//     each phase books its nights round-robin, a night's next reservation coming a whole round
//     (about the phase's night count over the reservation rate, several seconds) after the last.
//   - A search that misses the reservation cache fills it from the database after reading it,
//     which can overwrite a concurrent reservation's newer count (the same race). Each phase
//     therefore starts with PrefixOps operations without any reservation of a night (its
//     reservations become wrong-password probes, which never reach the reservation service),
//     whose searches go round every night booked so far and so fill the cache again after a fault
//     emptied it.
//   - Searches and reservations are of one night: the reservation service's availability check
//     of a multi-night stay answers from whichever night's database read finishes last when the
//     cache misses, which no model of the application's state can predict.
//   - After its plans, a phase probes its own nights again, and from the second phase on also the
//     nights of earlier phases (across the faults between phases), with one-room reservations.
//   - Logins and reservations use a wrong password in a small, fixed share of requests; the
//     frontend must refuse them (and must not book anything).
const (
	AccuracyPhases = 4
	NightsPerPhase = 3
	// PrefixOps is the operations at the start of a phase without a reservation of a night: three
	// seconds at the accuracy rate.
	PrefixOps = 90
	// FirstNight is the first night booked (April 9, the seeded reservation's night: hotel 4 has
	// one room booked on it).
	FirstNight = 9
	// unratedPerNight is how many hotels a search never returns get a plan on each night too.
	unratedPerNight = 3
	// wrongReserve is one in how many reservations (after the prefix) uses a wrong password;
	// wrongLogin the same for logins; oldProbePercent the share of reservations, from the second
	// phase on, that probe an earlier phase's night.
	wrongReserve    = 20
	wrongLogin      = 8
	oldProbePercent = 15
)

// The accuracy mix: searches observe the nights the reservations book; recommendations and logins
// are checked on their own.
const (
	accSearch    = 40
	accReserve   = 36
	accRecommend = 12
	accLogin     = 12
)

// Plan kinds: how a night's four reservations go, with f the rooms free when the phase starts.
const (
	PlanExact   = "exact"   // a + b + (f-a-b): full; then 1 more: refused
	PlanPartial = "partial" // a + (f-1-a): one room left; 2 more: refused; 1 more: full
	PlanOver    = "over"    // a; then f-a+1: refused (one too many); f-a: full; 1 more: refused
	PlanLight   = "light"   // four small reservations: never near full
)

var planKinds = []string{PlanExact, PlanPartial, PlanOver, PlanLight}
var planWeights = []float64{3, 3, 2, 2}

// NightPlan is the reservations a phase sends for one hotel's night.
type NightPlan struct {
	Hotel, Night string
	Kind         string
	Rooms        []int
}

// Night returns night i of the accuracy run (0 is FirstNight), YYYY-MM-DD.
func Night(i int) string { return april(FirstNight + i) }

// AccModel is the accuracy workload's model: every phase's plans (the same in every phase of a
// sample, built from the sample's model stream), and the phase's progress through them.
type AccModel struct {
	data  *hotel.Dataset
	plans [][]NightPlan // by phase, in the order a round books them
	phase int
	// ops counts the operations built in this phase; reserved the planned reservations sent,
	// probes the probes of this phase's nights after the plans, old the probes of earlier nights.
	ops, reserved, probes, old int
	// searches of this phase's nights and of earlier ones.
	current, earlier int
}

// Plans returns phase p's plans (for tests and the README's numbers).
func (m *AccModel) Plans(p int) []NightPlan { return m.plans[p] }

// NewAccModel builds every phase's plans from the dataset and the sample's model stream.
func NewAccModel(data *hotel.Dataset, rand *workload.Rand) (*AccModel, error) {
	rated := data.Rated()
	var unrated []*hotel.Hotel
	for i := range data.Hotels {
		if !data.Hotels[i].Rated {
			unrated = append(unrated, &data.Hotels[i])
		}
	}
	if len(rated) == 0 || len(unrated) < unratedPerNight {
		return nil, fmt.Errorf("the dataset has %d rated and %d other hotels", len(rated), len(unrated))
	}
	m := &AccModel{data: data}
	for p := 0; p < AccuracyPhases; p++ {
		r := rand.Child("phase", p)
		var plans []NightPlan
		for j := 0; j < NightsPerPhase; j++ {
			night := Night(p*NightsPerPhase + j)
			hotels := append([]*hotel.Hotel(nil), rated...)
			// A few hotels no search returns: reservations alone observe them. The seeded night
			// (hotel 4 on April 9) is always one of them.
			picked := map[string]bool{}
			if seeded := data.Hotel("4"); seeded != nil && data.BookedRooms("4", night) > 0 {
				hotels, picked["4"] = append(hotels, seeded), true
			}
			for len(picked) < unratedPerNight {
				h := unrated[r.Below(len(unrated))]
				if !picked[h.ID] {
					hotels, picked[h.ID] = append(hotels, h), true
				}
			}
			for _, h := range hotels {
				free := h.Capacity - data.BookedRooms(h.ID, night)
				plans = append(plans, newPlan(h.ID, night, free, r))
			}
		}
		// Shuffle, so that a round does not book one night's hotels back to back.
		for i := len(plans) - 1; i > 0; i-- {
			k := r.Below(i + 1)
			plans[i], plans[k] = plans[k], plans[i]
		}
		m.plans = append(m.plans, plans)
	}
	return m, nil
}

func newPlan(hotelID, night string, free int, r *workload.Rand) NightPlan {
	p := NightPlan{Hotel: hotelID, Night: night, Kind: planKinds[r.Weighted(planWeights)]}
	if free < 8 {
		p.Kind = PlanLight
	}
	switch p.Kind {
	case PlanExact:
		a := 1 + r.Below(free-2)
		b := 1 + r.Below(free-a-1)
		p.Rooms = []int{a, b, free - a - b, 1}
	case PlanPartial:
		a := 1 + r.Below(free-2)
		p.Rooms = []int{a, free - 1 - a, 2, 1}
	case PlanOver:
		a := 1 + r.Below(free-1)
		p.Rooms = []int{a, free - a + 1, free - a, 1}
	default:
		most := max(1, free/8)
		p.Rooms = []int{1 + r.Below(most), 1 + r.Below(most), 1 + r.Below(most), 1 + r.Below(most)}
	}
	return p
}

// steps is the number of reservations of every plan.
const steps = 4

func (m *AccModel) begin(phase int) error {
	if phase >= len(m.plans) {
		return fmt.Errorf("phase %d: the accuracy workload has plans for %d phases", phase, len(m.plans))
	}
	m.phase, m.ops, m.reserved, m.probes, m.old, m.current, m.earlier = phase, 0, 0, 0, 0, 0, 0
	return nil
}

func reservation(night string, hotelID string, rooms, user int, valid bool) []workload.Step {
	password := hotel.Password(user)
	if !valid {
		password = hotel.Password((user + 1) % hotel.Users)
	}
	out, _ := hotel.NextDay(night)
	return []workload.Step{workload.Get("/reservation",
		workload.Query("inDate", night), workload.Query("outDate", out),
		workload.Query("hotelId", hotelID), workload.Query("customerName", hotel.Username(user)),
		workload.Query("username", hotel.Username(user)), workload.Query("password", password),
		workload.Query("number", strconv.Itoa(rooms)))}
}

func (m *AccModel) reserve(rand *workload.Rand) workload.Request {
	m.ops++
	user := rand.Below(hotel.Users)
	plans := m.plans[m.phase]
	info := hotel.Info{Kind: "reserve", User: user, Valid: true}
	switch {
	case m.ops <= PrefixOps || rand.Below(wrongReserve) == 0:
		// A wrong password: refused before the reservation service is asked.
		p := plans[rand.Below(len(plans))]
		info.Hotel, info.Night, info.Rooms, info.Valid, info.Plan = p.Hotel, p.Night, 1, false, "wrong"
	case m.phase > 0 && rand.Below(100) < oldProbePercent:
		// One room on a night of an earlier phase, round-robin over all of them.
		n := 0
		for p := 0; p < m.phase; p++ {
			n += len(m.plans[p])
		}
		i := m.old % n
		m.old++
		for p := 0; p < m.phase; p++ {
			if i < len(m.plans[p]) {
				info.Hotel, info.Night = m.plans[p][i].Hotel, m.plans[p][i].Night
				break
			}
			i -= len(m.plans[p])
		}
		info.Rooms, info.Plan = 1, "old"
	case m.reserved < steps*len(plans):
		// The plans, a round at a time: step s of every night, then step s+1.
		s, p := m.reserved/len(plans), plans[m.reserved%len(plans)]
		m.reserved++
		info.Hotel, info.Night, info.Rooms, info.Plan, info.Step = p.Hotel, p.Night, p.Rooms[s], p.Kind, s+1
	default:
		// Past the plans: one room on this phase's nights again, in the plans' order.
		p := plans[m.probes%len(plans)]
		m.probes++
		info.Hotel, info.Night, info.Rooms, info.Plan = p.Hotel, p.Night, 1, "probe"
	}
	return workload.Request{Steps: reservation(info.Night, info.Hotel, info.Rooms, user, info.Valid), Info: info}
}

func (m *AccModel) search(rand *workload.Rand) workload.Request {
	m.ops++
	var night string
	booked := (m.phase + 1) * NightsPerPhase // nights booked so far, this phase's included
	switch {
	case m.ops <= PrefixOps:
		// Round every night booked so far, to fill the reservation cache before any reservation.
		night = Night(m.current % booked)
		m.current++
	case m.phase == 0 || rand.Below(10) < 7:
		night = Night(m.phase*NightsPerPhase + m.current%NightsPerPhase)
		m.current++
	default:
		night = Night(m.earlier % (m.phase * NightsPerPhase))
		m.earlier++
	}
	out, _ := hotel.NextDay(night)
	lat, lon := location(rand)
	return workload.Request{
		Steps: []workload.Step{workload.Get("/hotels",
			workload.Query("inDate", night), workload.Query("outDate", out),
			workload.Query("lat", coord(lat)), workload.Query("lon", coord(lon)))},
		Info: hotel.Info{Kind: "search", Night: night},
	}
}

func (m *AccModel) recommend(rand *workload.Rand) workload.Request {
	m.ops++
	require := requireOf(rand)
	lat, lon := location(rand)
	// A location two hotels are (nearly) equally near to has no single right answer to "dis":
	// draw another.
	for try := 0; require == "dis" && try < 20; try++ {
		la, _ := strconv.ParseFloat(coord(lat), 64)
		lo, _ := strconv.ParseFloat(coord(lon), 64)
		if _, ambiguous := m.data.Recommend(require, la, lo); !ambiguous {
			break
		}
		lat, lon = location(rand)
	}
	return workload.Request{
		Steps: []workload.Step{workload.Get("/recommendations",
			workload.Query("require", require), workload.Query("lat", coord(lat)), workload.Query("lon", coord(lon)))},
		Info: hotel.Info{Kind: "recommend", Require: require, Lat: coord(lat), Lon: coord(lon)},
	}
}

func (m *AccModel) login(rand *workload.Rand) workload.Request {
	m.ops++
	user := rand.Below(hotel.Users)
	valid := rand.Below(wrongLogin) != 0
	password := hotel.Password(user)
	if !valid {
		password = hotel.Password((user + 1) % hotel.Users)
	}
	return workload.Request{
		Steps: []workload.Step{workload.Get("/user",
			workload.Query("username", hotel.Username(user)), workload.Query("password", password))},
		Info: hotel.Info{Kind: "login", User: user, Valid: valid},
	}
}

// Accuracy is the accuracy check's workload.
func Accuracy() workload.Workload[*hotel.Dataset, *AccModel] {
	return workload.Workload[*hotel.Dataset, *AccModel]{
		Dataset:    hotel.ReadDataset,
		Model:      NewAccModel,
		BeginPhase: func(m *AccModel, phase int) error { return m.begin(phase) },
		Operations: []workload.Operation[*AccModel]{
			{Name: "search", Weight: accSearch, Build: (*AccModel).search},
			{Name: "reserve", Weight: accReserve, Build: (*AccModel).reserve},
			{Name: "recommend", Weight: accRecommend, Build: (*AccModel).recommend},
			{Name: "login", Weight: accLogin, Build: (*AccModel).login},
		},
	}
}
