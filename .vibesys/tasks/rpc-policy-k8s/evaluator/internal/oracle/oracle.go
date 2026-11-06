// Package oracle is the task's accuracy oracle: the specs a captured history of the accuracy
// workload must hold (package check of vibesys-evaluator).
//
// Two specs:
//
//   - "rooms" models the reservation state, one key per hotel night: the rooms booked, from the
//     seeded count. A reservation of n rooms with a right password succeeds exactly when the night
//     has n rooms left (and then books them) and is refused ("Already reserved") otherwise; a
//     search of the night shows the hotel exactly while it has a room left. An answer that is lost
//     or a 5xx may or may not have booked (within EffectWindow). The application keeps the count
//     in MongoDB and caches it in Redis, so restarts and cache flushes change nothing: the model
//     has no Fault.
//   - "responses" checks every answer on its own: the status, the body's shape, the hotels'
//     profiles, logins, wrong-password reservations and recommendations (which depend on the seeded
//     data only); and it counts every call once toward the share of unknown outcomes the check
//     allows (check.Options.MaxLostShare), whatever its kind.
package oracle

import (
	"bytes"
	"encoding/json"
	"fmt"
	"iter"
	"slices"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/uw-syfi/vibesys/sdk/vibesys-evaluator/go/check"

	"rpc-policy-k8s/evaluator/internal/hotel"
)

// EffectWindow bounds when a reservation whose answer was lost may take effect.
const EffectWindow = 10 * time.Second

// Specs is the oracle: the specs a history must hold, over the dataset the generator read.
func Specs(h *check.History) ([]check.AnySpec, error) {
	data := hotel.Seeded()
	if h.Dataset != "" {
		var err error
		if data, err = hotel.ReadDataset(h.Dataset); err != nil {
			return nil, err
		}
	}
	return []check.AnySpec{Rooms(data), Responses(data)}, nil
}

// ------------------------------------------------------------------ rooms

// NightIn is a piece of the rooms spec: a reservation of Rooms rooms, or a search's view of the
// night (Rooms 0), on a night of Capacity rooms.
type NightIn struct {
	Rooms    int
	Capacity int
}

// Rooms is the reservation state's spec. Its state is the rooms booked on one hotel's night; a
// piece's output is whether the reservation succeeded or the search showed the hotel.
func Rooms(data *hotel.Dataset) *check.Spec[int, NightIn, bool] {
	return &check.Spec[int, NightIn, bool]{
		Name: "rooms",
		Project: func(phases iter.Seq[[]check.Call], out *check.Projection[NightIn, bool]) error {
			return projectRooms(data, phases, out)
		},
		Init: func(keys []string) int {
			booked := 0
			for _, k := range keys {
				h, night, _ := hotel.SplitKey(k)
				booked += data.BookedRooms(h, night)
			}
			return booked
		},
		Step:         stepRooms,
		Equal:        func(a, b int) bool { return a == b },
		Hash:         func(s int) uint64 { return uint64(s) },
		EffectWindow: EffectWindow,
		DescribeState: func(s int) string {
			return fmt.Sprintf("%d booked", s)
		},
		DescribeOp: func(op check.Op[NightIn, bool]) string {
			if !op.Write {
				return fmt.Sprintf("search: %s", map[bool]string{true: "shown", false: "not shown"}[op.Out])
			}
			outcome := map[bool]string{true: "booked", false: "refused"}[op.Out]
			if op.Unknown {
				outcome = "unknown"
			}
			return fmt.Sprintf("reserve %d of %d: %s", op.In.Rooms, op.In.Capacity, outcome)
		},
	}
}

func stepRooms(booked int, op check.Op[NightIn, bool]) ([]int, string) {
	in := op.In
	if !op.Write {
		available := booked+1 <= in.Capacity
		switch {
		case op.Out == available:
			return []int{booked}, ""
		case op.Out:
			return nil, fmt.Sprintf("a search shows the hotel although all %d rooms of the night are booked", in.Capacity)
		default:
			return nil, fmt.Sprintf("a search leaves the hotel out although only %d of its %d rooms are booked", booked, in.Capacity)
		}
	}
	fits := booked+in.Rooms <= in.Capacity
	switch {
	case op.Unknown && fits:
		return []int{booked, booked + in.Rooms}, ""
	case op.Unknown, !op.Out && !fits:
		return []int{booked}, ""
	case op.Out && fits:
		return []int{booked + in.Rooms}, ""
	case op.Out:
		return nil, fmt.Sprintf("booked %d rooms with %d of %d already booked", in.Rooms, booked, in.Capacity)
	default:
		return nil, fmt.Sprintf("refused %d rooms with only %d of %d booked", in.Rooms, booked, in.Capacity)
	}
}

// projectRooms adds a piece per reservation with a right password (a write; unknown when the
// answer was lost or a 5xx) and, for every search answered, a read per hotel a search returns.
// Malformed answers add nothing: the responses spec reports them.
func projectRooms(data *hotel.Dataset, phases iter.Seq[[]check.Call], out *check.Projection[NightIn, bool]) error {
	rated := data.Rated()
	for calls := range phases {
		for i := range calls {
			c := &calls[i]
			info, err := infoOf(c)
			if err != nil {
				return err
			}
			unknown := unknownOutcome(c)
			switch c.Op {
			case "search":
				if unknown {
					continue // an unknown read tells nothing
				}
				shown, err := searchAnswer(data, c)
				if err != nil {
					continue
				}
				for _, h := range rated {
					out.Add(check.Piece[NightIn, bool]{
						Keys: []string{hotel.Key(h.ID, info.Night)}, In: NightIn{Capacity: h.Capacity},
						Out: shown[h.ID], Origin: c,
					})
				}
			case "reserve":
				if !info.Valid {
					continue // refused before the reservation service is asked
				}
				h := data.Hotel(info.Hotel)
				if h == nil {
					return fmt.Errorf("operation %d of phase %d reserves hotel %q, which is not in the dataset", c.Index, c.Phase, info.Hotel)
				}
				piece := check.Piece[NightIn, bool]{
					Keys: []string{hotel.Key(info.Hotel, info.Night)}, In: NightIn{Rooms: info.Rooms, Capacity: h.Capacity},
					Write: true, Unknown: unknown, Origin: c,
				}
				if !unknown {
					msg, err := message(c)
					if err != nil || (msg != hotel.MsgReserved && msg != hotel.MsgFull) {
						continue
					}
					piece.Out = msg == hotel.MsgReserved
				}
				out.Add(piece)
			}
		}
	}
	return nil
}

// ------------------------------------------------------------------ responses

// Responses checks every answer on its own and counts every call once: unknown when its answer
// was lost or a 5xx.
func Responses(data *hotel.Dataset) *check.Spec[struct{}, struct{}, struct{}] {
	return &check.Spec[struct{}, struct{}, struct{}]{
		Name: "responses",
		Project: func(phases iter.Seq[[]check.Call], out *check.Projection[struct{}, struct{}]) error {
			for calls := range phases {
				for i := range calls {
					c := &calls[i]
					info, err := infoOf(c)
					if err != nil {
						return err
					}
					unknown := unknownOutcome(c)
					out.Add(check.Piece[struct{}, struct{}]{Unknown: unknown, Origin: c})
					if unknown {
						continue
					}
					if problem := Judge(data, c, info); problem != "" {
						out.Violation(c, "%s", problem)
					}
				}
			}
			return nil
		},
		Init:  func([]string) struct{} { return struct{}{} },
		Step:  func(s struct{}, _ check.Op[struct{}, struct{}]) ([]struct{}, string) { return []struct{}{s}, "" },
		Equal: func(struct{}, struct{}) bool { return true },
		Hash:  func(struct{}) uint64 { return 0 },
	}
}

// Judge returns what is wrong with an answer on its own, or "" (an answer the rooms spec must
// still judge against the state, for searches and reservations).
func Judge(data *hotel.Dataset, c *check.Call, info hotel.Info) string {
	if len(c.Steps) != 1 {
		return fmt.Sprintf("%d steps; the workload sends one", len(c.Steps))
	}
	if status := c.Steps[0].Status; status != 200 {
		return fmt.Sprintf("status %d (%s): the frontend answers every request the workload sends with 200",
			status, snippet(c.Steps[0].Body))
	}
	switch c.Op {
	case "search":
		_, err := searchAnswer(data, c)
		if err != nil {
			return err.Error()
		}
	case "recommend":
		return judgeRecommendation(data, c, info)
	case "login":
		msg, err := message(c)
		if err != nil {
			return err.Error()
		}
		if want := map[bool]string{true: hotel.MsgLoginOK, false: hotel.MsgLoginFail}[info.Valid]; msg != want {
			return fmt.Sprintf("login of %s with a %s password: %q, want %q", hotel.Username(info.User), rightOrWrong(info.Valid), msg, want)
		}
	case "reserve":
		msg, err := message(c)
		if err != nil {
			return err.Error()
		}
		if !info.Valid && msg != hotel.MsgLoginFail {
			return fmt.Sprintf("reservation with a wrong password for %s: %q, want %q", hotel.Username(info.User), msg, hotel.MsgLoginFail)
		}
		if info.Valid && msg != hotel.MsgReserved && msg != hotel.MsgFull {
			return fmt.Sprintf("reservation with the right password: %q, want %q or %q", msg, hotel.MsgReserved, hotel.MsgFull)
		}
	default:
		return fmt.Sprintf("operation %q is not one of the accuracy workload's", c.Op)
	}
	return ""
}

func rightOrWrong(valid bool) string {
	if valid {
		return "right"
	}
	return "wrong"
}

func judgeRecommendation(data *hotel.Dataset, c *check.Call, info hotel.Info) string {
	ids, err := features(data, c.Steps[0].Body)
	if err != nil {
		return err.Error()
	}
	lat, err1 := strconv.ParseFloat(info.Lat, 64)
	lon, err2 := strconv.ParseFloat(info.Lon, 64)
	if err1 != nil || err2 != nil {
		return fmt.Sprintf("the operation's info has no location (%q, %q): a task bug", info.Lat, info.Lon)
	}
	allowed, ambiguous := data.Recommend(info.Require, lat, lon)
	got := map[string]bool{}
	for _, id := range ids {
		got[id] = true
	}
	ok := len(got) > 0
	for id := range got {
		ok = ok && allowed[id]
	}
	if !ambiguous {
		ok = ok && len(got) == len(allowed)
	}
	if !ok {
		return fmt.Sprintf("recommendations by %q at (%s, %s): hotels %v, want %v", info.Require, info.Lat, info.Lon, sortedKeys(got), sortedKeys(allowed))
	}
	return ""
}

// ------------------------------------------------------------------ answers

// unknownOutcome says whether a call's outcome is unknown: no answer, or a 5xx.
func unknownOutcome(c *check.Call) bool {
	if c.Lost() {
		return true
	}
	for _, s := range c.Steps {
		if s.Status >= 500 {
			return true
		}
	}
	return false
}

func infoOf(c *check.Call) (hotel.Info, error) {
	var info hotel.Info
	if err := json.Unmarshal(c.Info, &info); err != nil {
		return info, fmt.Errorf("operation %d of phase %d: its info %q: %w (a task bug)", c.Index, c.Phase, snippet(c.Info), err)
	}
	if info.Kind != c.Op {
		return info, fmt.Errorf("operation %d of phase %d is %q but its info says %q (a task bug)", c.Index, c.Phase, c.Op, info.Kind)
	}
	return info, nil
}

// message is a {"message": ...} answer's message.
func message(c *check.Call) (string, error) {
	if len(c.Steps) != 1 || c.Steps[0].Status != 200 {
		return "", fmt.Errorf("no answer")
	}
	var m struct {
		Message *string `json:"message"`
	}
	dec := json.NewDecoder(bytes.NewReader(c.Steps[0].Body))
	dec.DisallowUnknownFields()
	if err := dec.Decode(&m); err != nil || m.Message == nil {
		return "", fmt.Errorf("the body is not a {\"message\": ...} object: %s", snippet(c.Steps[0].Body))
	}
	return *m.Message, nil
}

// searchAnswer reads a search's answer: which hotels it shows. Every hotel must be one a search
// returns, at most once, with its profile.
func searchAnswer(data *hotel.Dataset, c *check.Call) (map[string]bool, error) {
	if len(c.Steps) != 1 || c.Steps[0].Status != 200 {
		return nil, fmt.Errorf("no answer")
	}
	ids, err := features(data, c.Steps[0].Body)
	if err != nil {
		return nil, err
	}
	shown := map[string]bool{}
	for _, id := range ids {
		if !data.Hotel(id).Rated {
			return nil, fmt.Errorf("a search shows hotel %s, which has no rate plan (a search returns rated hotels only)", id)
		}
		shown[id] = true
	}
	return shown, nil
}

type geoJSON struct {
	Features []struct {
		Geometry struct {
			Coordinates []json.Number `json:"coordinates"`
			Type        string        `json:"type"`
		} `json:"geometry"`
		ID         string `json:"id"`
		Properties struct {
			Name        string `json:"name"`
			PhoneNumber string `json:"phone_number"`
		} `json:"properties"`
		Type string `json:"type"`
	} `json:"features"`
	Type string `json:"type"`
}

// features reads a GeoJSON answer (a search's or a recommendation's) and checks each hotel's
// profile against the dataset; it returns the hotels' ids, each at most once.
func features(data *hotel.Dataset, body []byte) ([]string, error) {
	var g geoJSON
	dec := json.NewDecoder(bytes.NewReader(body))
	dec.DisallowUnknownFields()
	dec.UseNumber()
	if err := dec.Decode(&g); err != nil || g.Type != "FeatureCollection" || g.Features == nil {
		return nil, fmt.Errorf("the body is not a GeoJSON FeatureCollection: %s", snippet(body))
	}
	seen := map[string]bool{}
	var ids []string
	for _, f := range g.Features {
		h := data.Hotel(f.ID)
		switch {
		case h == nil:
			return nil, fmt.Errorf("hotel %q is not one of the %d hotels", f.ID, len(data.Hotels))
		case seen[f.ID]:
			return nil, fmt.Errorf("hotel %s appears twice", f.ID)
		case f.Type != "Feature" || f.Geometry.Type != "Point" || len(f.Geometry.Coordinates) != 2:
			return nil, fmt.Errorf("hotel %s is not a GeoJSON Point feature: %s", f.ID, snippet(body))
		case f.Properties.Name != h.Name || f.Properties.PhoneNumber != h.Phone:
			return nil, fmt.Errorf("hotel %s is %q, %q; its profile is %q, %q", f.ID, f.Properties.Name, f.Properties.PhoneNumber, h.Name, h.Phone)
		}
		lon, err1 := strconv.ParseFloat(string(f.Geometry.Coordinates[0]), 32)
		lat, err2 := strconv.ParseFloat(string(f.Geometry.Coordinates[1]), 32)
		if err1 != nil || err2 != nil || float32(lon) != h.Lon || float32(lat) != h.Lat {
			return nil, fmt.Errorf("hotel %s is at %v; its profile is at [%v, %v]", f.ID, f.Geometry.Coordinates, h.Lon, h.Lat)
		}
		seen[f.ID] = true
		ids = append(ids, f.ID)
	}
	return ids, nil
}

func snippet(b []byte) string {
	s := strings.TrimSpace(string(b))
	if len(s) > 200 {
		s = s[:200] + "..."
	}
	return strconv.Quote(s)
}

func sortedKeys(m map[string]bool) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Slice(keys, func(i, j int) bool {
		a, _ := strconv.Atoi(keys[i])
		b, _ := strconv.Atoi(keys[j])
		return a < b
	})
	return slices.Clip(keys)
}
