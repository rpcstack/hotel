package oracle

import (
	"fmt"
	"io"
	"sort"

	"github.com/uw-syfi/vibesys/sdk/vibesys-evaluator/go/check"

	"rpc-policy-k8s/evaluator/internal/hotel"
)

// Stats writes what a history exercised: the calls by operation and outcome, the reservations
// booked and refused, and how many searches left out a hotel because its night was full. It is
// how the README's coverage numbers are made (cmd/oracle stats).
func Stats(h *check.History, w io.Writer) error {
	data := hotel.Seeded()
	if h.Dataset != "" {
		var err error
		if data, err = hotel.ReadDataset(h.Dataset); err != nil {
			return err
		}
	}
	count := map[string]int{}
	var hidden, shown, searches int
	nights := map[string]bool{}
	for p := 0; p < h.Phases; p++ {
		calls, err := h.Phase(p)
		if err != nil {
			return err
		}
		for i := range calls {
			c := &calls[i]
			info, err := infoOf(c)
			if err != nil {
				return err
			}
			key := c.Op
			switch {
			case unknownOutcome(c):
				key += " unknown"
			case c.Op == "reserve" && !info.Valid:
				key += " wrong password"
			case c.Op == "reserve":
				m, _ := message(c)
				key += map[string]string{hotel.MsgReserved: " booked", hotel.MsgFull: " refused"}[m] + " (" + info.Plan + ")"
				nights[hotel.Key(info.Hotel, info.Night)] = true
			case c.Op == "login" && !info.Valid:
				key += " wrong password"
			case c.Op == "search":
				ids, err := searchAnswer(data, c)
				if err == nil {
					searches++
					shown += len(ids)
					hidden += len(data.Rated()) - len(ids)
				}
			}
			count[fmt.Sprintf("phase %d: %s", p, key)]++
			count["all: "+key]++
		}
	}
	keys := make([]string, 0, len(count))
	for k := range count {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	for _, k := range keys {
		fmt.Fprintf(w, "%-50s %6d\n", k, count[k])
	}
	fmt.Fprintf(w, "hotel nights reserved: %d\n", len(nights))
	fmt.Fprintf(w, "searches answered: %d, hotels shown %d, left out (night full) %d\n", searches, shown, hidden)
	return nil
}
