package hotel

import (
	"reflect"
	"sort"
	"strconv"
	"testing"
)

func ids(m map[string]bool) []int {
	var out []int
	for id := range m {
		n, _ := strconv.Atoi(id)
		out = append(out, n)
	}
	sort.Ints(out)
	return out
}

// The seeds as the application at the task's commit answers them (checked against a deployment:
// a search, the three recommendations at DeathStarBench's centre, a login).
func TestSeeded(t *testing.T) {
	d := Seeded()
	if err := d.Validate(); err != nil {
		t.Fatal(err)
	}
	var rated []int
	for _, h := range d.Rated() {
		n, _ := strconv.Atoi(h.ID)
		rated = append(rated, n)
		if h.Capacity != 200 {
			t.Errorf("hotel %s has %d rooms", h.ID, h.Capacity)
		}
	}
	want := []int{1, 2, 3, 9, 12, 15, 18, 21, 24, 27, 30, 33, 36, 39, 42, 45, 48, 51, 54, 57, 60, 63, 66, 69, 72, 75, 78}
	if !reflect.DeepEqual(rated, want) {
		t.Fatalf("rated %v", rated)
	}
	if h := d.Hotel("24"); h.Name != "St. Regis San Francisco" || h.Phone != "(415) 284-4024" || h.Lat != float32(37.9275) || h.Lon != float32(-122.218) {
		t.Fatalf("hotel 24: %+v", h)
	}
	for require, want := range map[string][]int{"rate": {9, 24, 39, 54, 69}, "price": {2}, "dis": {40}} {
		got, ambiguous := d.Recommend(require, 38.0235, -122.095)
		if ambiguous || !reflect.DeepEqual(ids(got), want) {
			t.Errorf("%s: %v (ambiguous %v), want %v", require, ids(got), ambiguous, want)
		}
	}
	if d.BookedRooms("4", "2015-04-09") != 1 || Password(12) != "12121212121212121212" {
		t.Fatal("seeded booking or password")
	}
}
