// Package hotel is the rpcstack hotel application as the task sees it: the data its services
// seed themselves with on startup (crates/hotel/src/*/db.rs and config/hotel.json at the task's
// pinned commit), the messages and routes of its HTTP frontend (crates/hotel/src/frontend/http.rs),
// and reference answers for the requests that depend on that data alone.
//
// The services write this data themselves, so the task's dataset is not written by a seed: it is
// what the application must hold, and the task's seed step checks that it does (Verify).
package hotel

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"strconv"
	"strings"
	"time"
)

// Messages of the HTTP frontend (crates/hotel/src/frontend/http.rs).
const (
	MsgLoginOK   = "Login successfully!"
	MsgLoginFail = "Failed. Please check your username and password. "
	MsgReserved  = "Reserve successfully!"
	MsgFull      = "Failed. Already reserved. "
)

// Users is config/hotel.json's user.users: accounts Username_0 .. Username_<Users-1>.
const Users = 1000

// Username and Password are the seeded account i (crates/hotel/src/user/server.rs): the password
// is i repeated ten times, as in DeathStarBench.
func Username(i int) string { return fmt.Sprintf("Username_%d", i) }
func Password(i int) string { return strings.Repeat(strconv.Itoa(i), 10) }

// Hotel is one of the 80 seeded hotels.
type Hotel struct {
	ID string `json:"id"`
	// Capacity is reservation-db's numberOfRoom: the rooms a night has.
	Capacity int `json:"capacity"`
	// Name, Phone, Lat and Lon are profile-db's: what a GeoJSON feature of the hotel shows. The
	// coordinates are float32, as the profile service stores them and the frontend encodes them.
	Name  string  `json:"name"`
	Phone string  `json:"phone"`
	Lat   float32 `json:"lat"`
	Lon   float32 `json:"lon"`
	// Rec is the recommendation service's in-memory record of the hotel.
	Rec Rec `json:"rec"`
	// Rated says whether rate-db has a plan for the hotel. A search returns rated hotels only:
	// the rate service answers a cache miss with its whole inventory, so every search considers
	// every rated hotel, wherever it is.
	Rated bool `json:"rated"`
}

// Rec is a hotel as the recommendation service holds it (recommendation/db.rs).
type Rec struct {
	Lat   float64 `json:"lat"`
	Lon   float64 `json:"lon"`
	Rate  float64 `json:"rate"`
	Price float64 `json:"price"`
}

// Booking is a seeded reservation (reservation/db.rs).
type Booking struct {
	Hotel string `json:"hotel"`
	Night string `json:"night"` // YYYY-MM-DD, the stay's in date (one night)
	Rooms int    `json:"rooms"`
}

// Dataset is everything the application holds when it starts: the task's dataset file.
type Dataset struct {
	Hotels []Hotel   `json:"hotels"`
	Users  int       `json:"users"`
	Booked []Booking `json:"booked"`
	index  map[string]int
}

// Seeded is the application's seed data at the task's pinned commit.
func Seeded() *Dataset {
	d := &Dataset{Users: Users, Booked: []Booking{{Hotel: "4", Night: "2015-04-09", Rooms: 1}}}
	type six struct {
		name, phone    string
		lat, lon       float32 // profile
		recLat, recLon float64 // recommendation
		rate, price    float64
	}
	first := []six{
		{"Clift Hotel", "(415) 775-4700", 37.7867, -122.4112, 37.7867, -122.4112, 109.00, 150.00},
		{"W San Francisco", "(415) 777-5300", 37.7854, -122.4005, 37.7854, -122.4005, 139.00, 120.00},
		{"Hotel Zetta", "(415) 543-8555", 37.7834, -122.4071, 37.7834, -122.4071, 109.00, 190.00},
		{"Hotel Vitale", "(415) 278-3700", 37.7936, -122.3930, 37.7936, -122.3930, 129.00, 160.00},
		{"Phoenix Hotel", "(415) 776-1380", 37.7831, -122.4181, 37.7831, -122.4181, 119.00, 140.00},
		{"St. Regis San Francisco", "(415) 284-4000", 37.7863, -122.4015, 37.7863, -122.4015, 149.00, 200.00},
	}
	for i := 1; i <= 80; i++ {
		h := Hotel{ID: strconv.Itoa(i)}
		// reservation/db.rs: hotels 1-6 have 200 rooms; then 300, 250 or 200 by i mod 3.
		h.Capacity = 200
		if i > 6 {
			h.Capacity = map[int]int{1: 300, 2: 250, 0: 200}[i%3]
		}
		// rate/db.rs: plans for hotels 1, 2 and 3, and for every multiple of 3 from 9 to 78.
		h.Rated = i <= 3 || (i >= 7 && i%3 == 0)
		if i <= 6 {
			s := first[i-1]
			h.Name, h.Phone, h.Lat, h.Lon = s.name, s.phone, s.lat, s.lon
			h.Rec = Rec{Lat: s.recLat, Lon: s.recLon, Rate: s.rate, Price: s.price}
		} else {
			h.Name = "St. Regis San Francisco"
			h.Phone = fmt.Sprintf("(415) 284-40%d", i)
			// profile/db.rs computes in f32: 37.7835 + (i as f32) / 500.0 * 3.0. Each
			// conversion rounds to float32, as Rust does after every operation.
			fi := float32(i)
			h.Lat = float32(float32(37.7835) + float32(float32(fi/500.0)*3.0))
			h.Lon = float32(float32(-122.41) + float32(float32(fi/500.0)*4.0))
			// recommendation/db.rs computes in f64.
			rate, price := 135.00, 179.00
			if i%3 == 0 {
				rate, price = recByMod5[i%5][0], recByMod5[i%5][1]
			}
			h.Rec = Rec{Lat: 37.7835 + float64(i)/500.0*3.0, Lon: -122.41 + float64(i)/500.0*4.0, Rate: rate, Price: price}
		}
		d.Hotels = append(d.Hotels, h)
	}
	return d
}

// recByMod5 is recommendation/db.rs's (rate, price) of a hotel past 6 whose id is a multiple of 3,
// by id mod 5.
var recByMod5 = [5][2]float64{{109.00, 123.17}, {120.00, 140.00}, {124.00, 144.00}, {132.00, 158.00}, {232.00, 258.00}}

// Hotel returns the hotel with id, or nil.
func (d *Dataset) Hotel(id string) *Hotel {
	if d.index == nil {
		d.index = map[string]int{}
		for i := range d.Hotels {
			d.index[d.Hotels[i].ID] = i
		}
	}
	if i, ok := d.index[id]; ok {
		return &d.Hotels[i]
	}
	return nil
}

// Rated returns the hotels a search considers, in id order.
func (d *Dataset) Rated() []*Hotel {
	var out []*Hotel
	for i := range d.Hotels {
		if d.Hotels[i].Rated {
			out = append(out, &d.Hotels[i])
		}
	}
	return out
}

// BookedRooms returns the seeded rooms of a hotel's night.
func (d *Dataset) BookedRooms(hotel, night string) int {
	n := 0
	for _, b := range d.Booked {
		if b.Hotel == hotel && b.Night == night {
			n += b.Rooms
		}
	}
	return n
}

// Key is the state key of a hotel's night: "<hotel>@<YYYY-MM-DD>".
func Key(hotel, night string) string { return hotel + "@" + night }

// SplitKey is Key's inverse.
func SplitKey(key string) (hotel, night string, ok bool) { return strings.Cut(key, "@") }

// Validate checks a dataset read from a file.
func (d *Dataset) Validate() error {
	if len(d.Hotels) == 0 || d.Users <= 0 {
		return fmt.Errorf("the dataset has %d hotels and %d users", len(d.Hotels), d.Users)
	}
	seen := map[string]bool{}
	for _, h := range d.Hotels {
		if seen[h.ID] || h.Capacity <= 0 {
			return fmt.Errorf("hotel %q: duplicate or without capacity", h.ID)
		}
		seen[h.ID] = true
	}
	return nil
}

// ------------------------------------------------------------------ recommendations

// earthRadius is geo 0.28's MEAN_EARTH_RADIUS (metres).
const earthRadius = 6371008.8

// degToRad is Rust's f64::to_radians factor, PI / 180 computed in float64 (not as Go's exact
// constant), so the distances round as the service's do.
var pi = math.Pi
var degToRad = pi / 180

// haversine is geo 0.28's Point::haversine_distance, divided by 1000 as the service does.
func haversine(lat1, lon1, lat2, lon2 float64) float64 {
	theta1, theta2 := lat1*degToRad, lat2*degToRad
	dTheta, dLambda := (lat2-lat1)*degToRad, (lon2-lon1)*degToRad
	s1, s2 := math.Sin(dTheta/2), math.Sin(dLambda/2)
	a := s1*s1 + math.Cos(theta1)*math.Cos(theta2)*(s2*s2)
	return earthRadius * (2 * math.Asin(math.Sqrt(a))) / 1000
}

// tieTolerance is how close (relatively) two distances must be for the reference to treat them
// as a tie: the service compares them exactly in its own floating point, which may round a few
// units in the last place differently from Go's.
const tieTolerance = 1e-9

// Recommend is the recommendation service's answer (recommendation/server.rs get_recs): the hotels
// nearest to (lat, lon) for "dis", of the highest rate for "rate", of the lowest price for
// "price". allowed holds every hotel the service may return (ties within tieTolerance for "dis");
// ambiguous says whether two hotels are that close to tied, so a generator can avoid the location.
func (d *Dataset) Recommend(require string, lat, lon float64) (allowed map[string]bool, ambiguous bool) {
	allowed = map[string]bool{}
	switch require {
	case "dis":
		best, second := math.MaxFloat64, math.MaxFloat64
		dist := make([]float64, len(d.Hotels))
		for i, h := range d.Hotels {
			dist[i] = haversine(lat, lon, h.Rec.Lat, h.Rec.Lon)
			if dist[i] < best {
				best, second = dist[i], best
			} else if dist[i] < second {
				second = dist[i]
			}
		}
		for i, h := range d.Hotels {
			if dist[i] <= best*(1+tieTolerance) {
				allowed[h.ID] = true
			}
		}
		return allowed, second <= best*(1+tieTolerance)
	case "rate":
		top := 0.0
		for _, h := range d.Hotels {
			top = math.Max(top, h.Rec.Rate)
		}
		for _, h := range d.Hotels {
			if h.Rec.Rate == top {
				allowed[h.ID] = true
			}
		}
	case "price":
		low := math.MaxFloat64
		for _, h := range d.Hotels {
			low = math.Min(low, h.Rec.Price)
		}
		for _, h := range d.Hotels {
			if h.Rec.Price == low {
				allowed[h.ID] = true
			}
		}
	}
	return allowed, false
}

// ------------------------------------------------------------------ operations

// Info is what the accuracy workload records with every operation it sends (workload.Request's
// Info): what was asked, which the oracle needs and the captured history does not otherwise have.
type Info struct {
	// Kind is the operation: search, reserve, recommend or login.
	Kind string `json:"kind"`
	// Night is a search's or reservation's night (inDate; outDate is the day after).
	Night string `json:"night,omitempty"`
	// Hotel and Rooms are a reservation's.
	Hotel string `json:"hotel,omitempty"`
	Rooms int    `json:"rooms,omitempty"`
	// User is the account a login or reservation uses; Valid says whether its password is right.
	User  int  `json:"user"`
	Valid bool `json:"valid"`
	// Plan says why a reservation was sent (for people reading a failure): a step of a night's
	// plan (exact, partial, over, light), a probe of a night of this phase or an earlier one, or a
	// wrong-password probe.
	Plan string `json:"plan,omitempty"`
	Step int    `json:"step,omitempty"`
	// Require, Lat and Lon are a recommendation's, the coordinates exactly as sent.
	Require string `json:"require,omitempty"`
	Lat     string `json:"lat,omitempty"`
	Lon     string `json:"lon,omitempty"`
}

// NextDay is the day after night (YYYY-MM-DD), a stay's outDate.
func NextDay(night string) (string, error) {
	t, err := time.Parse("2006-01-02", night)
	if err != nil {
		return "", err
	}
	return t.AddDate(0, 0, 1).Format("2006-01-02"), nil
}

// ReadDataset reads a dataset file (dataset.json: Seeded(), as encoding/json writes it).
func ReadDataset(path string) (*Dataset, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var d Dataset
	if err := json.Unmarshal(raw, &d); err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	if err := d.Validate(); err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	return &d, nil
}
