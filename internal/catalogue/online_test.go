//go:build online

// This is the build-time check design.md specifies and no other task carries:
// every address committed in the catalogue is requested, and the build fails
// naming the entry if one does not resolve.
//
// It is behind the `online` tag because it reaches the publisher's servers. A
// test that depends on somebody else's uptime should not be able to fail a
// developer's `go test ./...`, and this one measures their uptime as much as it
// measures this repository. Continuous integration runs it, which is where it
// belongs: the failure it catches is upstream renaming an image, and CI is
// where that should be heard rather than an operator partway through a 200 MiB
// download.
//
//	go test -tags=online ./internal/catalogue/

package catalogue

import (
	"context"
	"testing"
	"time"
)

// TestEveryCommittedAddressResolves runs the check over the real catalogue.
func TestEveryCommittedAddressResolves(t *testing.T) {
	// Generous next to the per-request timeout, because a handful of requests
	// across a build agent's connection is not slow when it works and is not
	// fast when it does not.
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()

	groups := addressGroups(Devices)
	failures := Check(ctx, nil, Devices)
	if len(failures) > 0 {
		t.Error(Report(failures))
		return
	}

	t.Logf("all %d image addresses resolved, covering %d devices", len(groups), len(Devices))
}
