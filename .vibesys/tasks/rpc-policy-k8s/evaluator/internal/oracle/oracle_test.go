package oracle

import (
	"encoding/json"
	"fmt"
	"strings"
	"testing"

	"github.com/uw-syfi/vibesys/sdk/vibesys-evaluator/go/check"

	"rpc-policy-k8s/evaluator/internal/hotel"
)

// history builds calls one after another, 10 ms apart, unless a test sets times itself.
type history struct {
	data  *hotel.Dataset
	calls []check.Call
	now   int64
}

func newHistory() *history { return &history{data: hotel.Seeded(), now: 1_000_000_000} }

func (h *history) add(op string, info hotel.Info, status int, body string) *check.Call {
	raw, _ := json.Marshal(info)
	start := h.now
	h.now += 10_000_000
	c := check.Call{
		Index: uint64(len(h.calls)), Client: 0, Op: op, Info: raw, Start: start, End: start + 5_000_000,
		Steps: []check.Step{{Start: start, End: start + 5_000_000, Status: status, Body: []byte(body)}},
	}
	if status == 0 {
		c.Steps[0].Error = "timeout"
	}
	h.calls = append(h.calls, c)
	return &h.calls[len(h.calls)-1]
}

func msg(m string) string { return fmt.Sprintf(`{"message":%q}`+"\n", m) }

func (h *history) reserve(hotelID, night string, rooms int, answer string) {
	h.add("reserve", hotel.Info{Kind: "reserve", Hotel: hotelID, Night: night, Rooms: rooms, User: 1, Valid: true}, 200, msg(answer))
}

// search answers with the rated hotels, except those in hidden.
func (h *history) search(night string, hidden ...string) {
	h.add("search", hotel.Info{Kind: "search", Night: night}, 200, geo(h.data, h.data.Rated(), hidden...))
}

func geo(data *hotel.Dataset, hotels []*hotel.Hotel, hidden ...string) string {
	var features []string
	for _, x := range hotels {
		skip := false
		for _, id := range hidden {
			skip = skip || id == x.ID
		}
		if skip {
			continue
		}
		features = append(features, fmt.Sprintf(`{"geometry":{"coordinates":[%v,%v],"type":"Point"},"id":%q,"properties":{"name":%q,"phone_number":%q},"type":"Feature"}`,
			x.Lon, x.Lat, x.ID, x.Name, x.Phone))
	}
	return `{"features":[` + strings.Join(features, ",") + `],"type":"FeatureCollection"}` + "\n"
}

func (h *history) check(t *testing.T) *check.Report {
	t.Helper()
	specs := []check.AnySpec{Rooms(h.data), Responses(h.data)}
	return check.Check(check.NewHistory([][]check.Call{h.calls}, nil), specs, check.Options{})
}

func expect(t *testing.T, r *check.Report, want check.Verdict, text string) {
	t.Helper()
	out := r.Text(10)
	if r.Verdict != want || !strings.Contains(out, text) {
		t.Fatalf("verdict %s, want %s with %q:\n%s", r.Verdict, want, text, out)
	}
}

const night = "2015-04-10"

func TestFillingANightPasses(t *testing.T) {
	h := newHistory()
	h.search(night)
	h.reserve("9", night, 150, hotel.MsgReserved)
	h.reserve("9", night, 51, hotel.MsgFull) // 201 > 200
	h.search(night)
	h.reserve("9", night, 50, hotel.MsgReserved) // exactly full
	h.reserve("9", night, 1, hotel.MsgFull)
	h.search(night, "9")
	// The seeded night: hotel 4 has 1 of 200 rooms booked on April 9.
	h.reserve("4", "2015-04-09", 199, hotel.MsgReserved)
	h.reserve("4", "2015-04-09", 1, hotel.MsgFull)
	expect(t, h.check(t), check.Pass, "PASS")
}

func TestSearchShowingAFullHotelFails(t *testing.T) {
	h := newHistory()
	h.reserve("12", night, 200, hotel.MsgReserved)
	h.search(night) // still shows 12: a stale answer
	expect(t, h.check(t), check.Fail, "all 200 rooms of the night are booked")
}

func TestSearchHidingAnAvailableHotelFails(t *testing.T) {
	h := newHistory()
	h.reserve("12", night, 199, hotel.MsgReserved)
	h.search(night, "12")
	expect(t, h.check(t), check.Fail, "only 199 of its 200 rooms are booked")
}

func TestRefusingAFreeNightFails(t *testing.T) {
	h := newHistory()
	h.reserve("12", night, 100, hotel.MsgReserved)
	h.reserve("12", night, 100, hotel.MsgFull) // 200 fits
	expect(t, h.check(t), check.Fail, "refused 100 rooms with only 100 of 200 booked")
}

func TestDoubleBookingFails(t *testing.T) {
	h := newHistory()
	// A policy that sends a reservation twice: the second 120 is refused although only 120 of
	// the 200 rooms were acknowledged.
	h.reserve("12", night, 60, hotel.MsgReserved)
	h.reserve("12", night, 60, hotel.MsgReserved)
	h.reserve("12", night, 80, hotel.MsgFull)
	expect(t, h.check(t), check.Fail, "refused 80 rooms")
}

func TestLostReservationMayHaveBooked(t *testing.T) {
	for _, booked := range []bool{true, false} {
		h := newHistory()
		h.add("reserve", hotel.Info{Kind: "reserve", Hotel: "12", Night: night, Rooms: 100, User: 1, Valid: true}, 0, "")
		h.reserve("12", night, 100, hotel.MsgReserved)
		if booked {
			h.reserve("12", night, 1, hotel.MsgFull)
		} else {
			h.reserve("12", night, 100, hotel.MsgReserved)
		}
		// Plenty of answered calls and pieces, so that the one lost stays under the lost share.
		for i := 0; i < 200; i++ {
			h.add("login", hotel.Info{Kind: "login", User: 3, Valid: true}, 200, msg(hotel.MsgLoginOK))
		}
		for i := 0; i < 10; i++ {
			h.search("2015-04-11")
		}
		expect(t, h.check(t), check.Pass, "PASS")
	}
}

func TestTooManyUnknownOutcomesFail(t *testing.T) {
	h := newHistory()
	for i := 0; i < 20; i++ {
		h.add("login", hotel.Info{Kind: "login", User: 3, Valid: true}, 200, msg(hotel.MsgLoginOK))
	}
	h.add("search", hotel.Info{Kind: "search", Night: night}, 503, "unavailable\n")
	expect(t, h.check(t), check.Fail, "responses")
}

func TestStaticViolations(t *testing.T) {
	data := hotel.Seeded()
	cases := []struct {
		name, op string
		info     hotel.Info
		status   int
		body     string
		want     string
	}{
		{"wrong password booked", "reserve", hotel.Info{Kind: "reserve", Hotel: "12", Night: night, Rooms: 1, User: 1},
			200, msg(hotel.MsgReserved), "wrong password"},
		{"wrong password logged in", "login", hotel.Info{Kind: "login", User: 1}, 200, msg(hotel.MsgLoginOK), "with a wrong password"},
		{"right password refused", "login", hotel.Info{Kind: "login", User: 1, Valid: true}, 200, msg(hotel.MsgLoginFail), "with a right password"},
		{"a 400", "login", hotel.Info{Kind: "login", User: 1, Valid: true}, 400, "Please specify username and password\n", "status 400"},
		{"unrated hotel in a search", "search", hotel.Info{Kind: "search", Night: night}, 200,
			geo(data, []*hotel.Hotel{data.Hotel("4")}), "no rate plan"},
		{"a wrong profile", "search", hotel.Info{Kind: "search", Night: night}, 200,
			strings.Replace(geo(data, []*hotel.Hotel{data.Hotel("9")}), "St. Regis", "St Regis", 1), "its profile is"},
		{"a hotel twice", "search", hotel.Info{Kind: "search", Night: night}, 200,
			geo(data, []*hotel.Hotel{data.Hotel("9"), data.Hotel("9")}), "appears twice"},
		{"not JSON", "search", hotel.Info{Kind: "search", Night: night}, 200, "<html>", "not a GeoJSON"},
		{"wrong recommendation", "recommend", hotel.Info{Kind: "recommend", Require: "price", Lat: "38.0235", Lon: "-122.0950"}, 200,
			geo(data, []*hotel.Hotel{data.Hotel("3")}), "want [2]"},
		{"a recommendation short", "recommend", hotel.Info{Kind: "recommend", Require: "rate", Lat: "38.0235", Lon: "-122.0950"}, 200,
			geo(data, []*hotel.Hotel{data.Hotel("9"), data.Hotel("24")}), "want [9 24 39 54 69]"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			h := newHistory()
			for i := 0; i < 200; i++ {
				h.add("login", hotel.Info{Kind: "login", User: 3, Valid: true}, 200, msg(hotel.MsgLoginOK))
			}
			h.add(c.op, c.info, c.status, c.body)
			expect(t, h.check(t), check.Fail, c.want)
		})
	}
}

func TestRightAnswersPass(t *testing.T) {
	data := hotel.Seeded()
	h := newHistory()
	h.add("recommend", hotel.Info{Kind: "recommend", Require: "price", Lat: "38.0235", Lon: "-122.0950"}, 200, geo(data, []*hotel.Hotel{data.Hotel("2")}))
	rate := []*hotel.Hotel{data.Hotel("69"), data.Hotel("9"), data.Hotel("24"), data.Hotel("54"), data.Hotel("39")}
	h.add("recommend", hotel.Info{Kind: "recommend", Require: "rate", Lat: "38.0235", Lon: "-122.0950"}, 200, geo(data, rate))
	// The service's answer at the frontend's default probe location (checked against the app).
	h.add("recommend", hotel.Info{Kind: "recommend", Require: "dis", Lat: "38.0235", Lon: "-122.0950"}, 200, geo(data, []*hotel.Hotel{data.Hotel("40")}))
	h.add("login", hotel.Info{Kind: "login", User: 7, Valid: false}, 200, msg(hotel.MsgLoginFail))
	h.add("reserve", hotel.Info{Kind: "reserve", Hotel: "12", Night: night, Rooms: 1, User: 1}, 200, msg(hotel.MsgLoginFail))
	expect(t, h.check(t), check.Pass, "PASS")
}
