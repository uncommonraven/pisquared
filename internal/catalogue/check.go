package catalogue

import (
	"context"
	"fmt"
	"net/http"
	"strings"
	"time"
)

// The build-time link check.
//
// design.md specifies that every address committed in the catalogue is checked
// in the build, and no other task carries it. This is that check: one HEAD
// request per distinct address, failing and naming the entry if one does not
// resolve.
//
// The failure it catches is upstream renaming or removing an image, which would
// otherwise reach an operator as a 404 partway through a 200 MiB download. The
// failure it does not catch is upstream adding a board, because nothing tells us
// to go looking for one. That asymmetry is recorded in design.md as accepted
// rather than solved, and is why the documentation page carries a manual route
// that does not depend on this table.

// checkTimeout bounds a single HEAD request. Ten seconds is generous for a CDN
// and short enough that a build reports a named link failure quickly instead of
// appearing to hang.
const checkTimeout = 10 * time.Second

// DefaultClient is the client Check uses when it is handed none. It is a
// function rather than a package variable so that a test, or a caller wanting a
// different policy, has no shared mutable client to race over.
func DefaultClient() *http.Client {
	return &http.Client{Timeout: checkTimeout}
}

// Failure is one address that did not resolve, and the devices that wanted it.
type Failure struct {
	// Address is the address that was requested and did not resolve.
	Address string

	// Devices are the catalogue entries that resolve to it. Several devices
	// share one image, so a failure names every name an operator might
	// recognise rather than only the first entry that happened to be checked.
	Devices []Device

	// Reason is what came back: the server's own status line, or the error the
	// request failed with. It is kept verbatim rather than reduced to "failed",
	// because the difference between a 404, a 403, and a timeout is the
	// difference between a renamed image, a refused request, and a network
	// that is down — and the fix for each is in a different place.
	Reason string
}

func (f Failure) Error() string {
	names := make([]string, 0, len(f.Devices))
	for _, d := range f.Devices {
		names = append(names, fmt.Sprintf("%s (%s)", d.Name, d.ID))
	}
	return fmt.Sprintf("%s does not resolve: %s. The catalogue sends %s to it.",
		f.Address, f.Reason, strings.Join(names, ", "))
}

// Check issues one HEAD request per distinct address in devices and returns
// every address that did not resolve, in the order the addresses first appear.
// It returns nil when they all resolve.
//
// Addresses are requested as they are committed rather than rewritten, so the
// code the build runs is the same code a test runs; redirecting an address at
// the transport layer is how a test reaches a local server without the
// catalogue growing a field that only a test would ever set.
func Check(ctx context.Context, client *http.Client, devices []Device) []Failure {
	if client == nil {
		client = DefaultClient()
	}
	var failures []Failure
	for _, g := range addressGroups(devices) {
		req, err := http.NewRequestWithContext(ctx, http.MethodHead, g.address, nil)
		if err != nil {
			failures = append(failures, Failure{Address: g.address, Devices: g.devices, Reason: err.Error()})
			continue
		}
		resp, err := client.Do(req)
		if err != nil {
			failures = append(failures, Failure{Address: g.address, Devices: g.devices, Reason: err.Error()})
			continue
		}
		// A HEAD response carries no body, but closing it is what releases the
		// connection. A check that leaked one per address would present as a
		// hang rather than as a named failure.
		resp.Body.Close()
		if resp.StatusCode != http.StatusOK {
			failures = append(failures, Failure{Address: g.address, Devices: g.devices, Reason: resp.Status})
		}
	}
	return failures
}

// Report names every address that did not resolve, in a form worth reading in
// build output, and returns the empty string when there was nothing to report.
func Report(failures []Failure) string {
	if len(failures) == 0 {
		return ""
	}
	var b strings.Builder
	fmt.Fprintf(&b, "%d of the catalogue's image addresses did not resolve:\n", len(failures))
	for _, f := range failures {
		fmt.Fprintf(&b, "  %s\n", f.Error())
	}
	return b.String()
}

// group is one distinct address and the catalogue entries that resolve to it.
type group struct {
	address string
	devices []Device
}

// addressGroups collapses devices to one entry per distinct address, keeping
// the order the addresses first appear in.
//
// Devices that share an image are checked once rather than once each. Fourteen
// requests would be ten more than the four images need, and they would all
// report the same thing at the same moment; a failure still names every device
// that wanted the address, so nothing is lost by asking once.
func addressGroups(devices []Device) []group {
	groups := make([]group, 0, len(devices))
	at := make(map[string]int, len(devices))
	for _, d := range devices {
		address := d.Address()
		if i, seen := at[address]; seen {
			groups[i].devices = append(groups[i].devices, d)
			continue
		}
		at[address] = len(groups)
		groups = append(groups, group{address: address, devices: []Device{d}})
	}
	return groups
}
