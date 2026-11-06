package task

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/uw-syfi/vibesys/sdk/vibesys-evaluator/go/k8s"
)

// The task's manifest renders, and names the App's Deployments, caches and gateway.
func TestManifestRenders(t *testing.T) {
	app := App()
	if err := app.Validate(); err != nil {
		t.Fatal(err)
	}
	text, err := os.ReadFile(filepath.Join("..", "..", "..", app.Manifest))
	if err != nil {
		t.Fatal(err)
	}
	r, err := k8s.Render(string(text), "hk8s-test", map[string]string{"HOTEL_IMAGE": "registry/x:1"}, nil, app.Manifest)
	if err != nil {
		t.Fatal(err)
	}
	kinds := map[string]int{}
	names := map[string]bool{}
	for _, o := range r.Objects() {
		kind, _ := o["kind"].(string)
		kinds[kind]++
		if kind == "Deployment" {
			meta, _ := o["metadata"].(map[string]any)
			name, _ := meta["name"].(string)
			names[name] = true
		}
	}
	if kinds["Deployment"] != 18 || kinds["Service"] != 18 || kinds["ConfigMap"] != 1 || len(kinds) != 3 {
		t.Fatalf("objects: %v", kinds)
	}
	for _, n := range append(app.Restartable, "reservation-redis", "profile-redis", "rate-redis") {
		if !names[n] {
			t.Errorf("no Deployment %s", n)
		}
	}
}

func TestMedian(t *testing.T) {
	if m := median([]float64{3, 1, 2}); m != 2 {
		t.Fatal(m)
	}
	if m := median([]float64{4, 1}); m != 2.5 {
		t.Fatal(m)
	}
}
