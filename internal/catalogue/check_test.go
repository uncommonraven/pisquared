package catalogue

import (
	"context"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync"
	"testing"
	"time"
)

// The tests here cover the build-time link check against a local server rather
// than the publisher's, so that `go test ./...` neither depends on the network
// nor fails when someone else's server is down. The check itself runs
// unmodified over the real addresses: the redirection happens below it, in the
// transport, so what runs here is the same code the build runs.
//
// The check over the committed catalogue, against the publisher's own servers,
// is online_test.go behind the `online` tag.

// testTimeout bounds these tests. They are all local, so a failure is a hang
// and this is only there to turn one into a named timeout. It is deliberately
// not the production bound: that one is set for a build talking to a CDN, and
// reusing it here would make these tests fail slowly instead of clearly.
const testTimeout = 2 * time.Second

// fixtureDevices is a small catalogue for the check to run over, pointed at a
// local server by the transport. Two of its three entries deliberately share
// an address, so every test here exercises the grouping rather than assuming
// one device means one request.
func fixtureDevices() []Device {
	return []Device{
		{ID: "alpha", Name: "Alpha Board", Board: "Alpha", Arch: "ARMv8", Codename: "Trixie"},
		{ID: "beta", Name: "Beta Board", Board: "Beta", Arch: "ARMv8", Codename: "Trixie"},
		{ID: "beta-late", Name: "Beta Board, later revision", Board: "Beta", Arch: "ARMv8", Codename: "Trixie"},
	}
}

// statusServer answers HEAD requests, returns the status configured for each
// path, and records what it was asked for so a test can assert on the requests
// as well as the outcome.
type statusServer struct {
	mu     sync.Mutex
	status map[string]int
	paths  []string
}

func (s *statusServer) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	s.mu.Lock()
	s.paths = append(s.paths, r.Method+" "+r.URL.Path)
	code, configured := s.status[r.URL.Path]
	s.mu.Unlock()

	if configured {
		w.WriteHeader(code)
		return
	}
	w.WriteHeader(http.StatusOK)
}

func (s *statusServer) requests() []string {
	s.mu.Lock()
	defer s.mu.Unlock()
	return append([]string(nil), s.paths...)
}

// serve starts the server and returns a client whose transport sends every
// request to it, keeping the path. The addresses under test still read as
// dietpi.com addresses, so the check is not given a special case for tests.
func serve(t *testing.T, status map[string]int) (*statusServer, *http.Client) {
	t.Helper()

	s := &statusServer{status: status}
	server := httptest.NewServer(s)
	t.Cleanup(server.Close)

	target, err := url.Parse(server.URL)
	if err != nil {
		t.Fatal(err)
	}
	return s, &http.Client{Transport: redirectTo{target: target}, Timeout: testTimeout}
}

// redirectTo sends every request to target instead of to wherever its URL
// points.
type redirectTo struct{ target *url.URL }

func (r redirectTo) RoundTrip(req *http.Request) (*http.Response, error) {
	// The request is cloned rather than modified because a RoundTripper must
	// not alter what it was handed, and the check's own request is what a
	// caller might inspect afterwards.
	out := req.Clone(req.Context())
	out.URL.Scheme = r.target.Scheme
	out.URL.Host = r.target.Host
	return http.DefaultTransport.RoundTrip(out)
}

// pathOf is the URL path of a device's address. It is taken from the address
// rather than rebuilt from the image name, because AddressPrefix carries the
// /downloads/images/ path and a stub matching only the filename would let a
// request fall through to a 200 and pass for a resolved address.
func pathOf(t *testing.T, d Device) string {
	t.Helper()
	parsed, err := url.Parse(d.Address())
	if err != nil {
		t.Fatal(err)
	}
	return parsed.Path
}

// TestCheckPassesWhenEveryAddressResolves is the passing half of task 3.5: a
// run in which every address resolves passes. A link check that only ever
// reports failures is indistinguishable from one wired up wrongly.
func TestCheckPassesWhenEveryAddressResolves(t *testing.T) {
	server, client := serve(t, nil)

	ctx, cancel := context.WithTimeout(context.Background(), testTimeout)
	defer cancel()

	devices := fixtureDevices()
	failures := Check(ctx, client, devices)

	if len(failures) != 0 {
		t.Fatalf("every address resolved and the check reported:\n%s", Report(failures))
	}
	if got := Report(failures); got != "" {
		t.Errorf("Report of no failures = %q, want the empty string", got)
	}

	// Three devices, two distinct addresses, two requests. Checking one image
	// once per device would be ten more requests for this catalogue and would
	// report the same failure as a row of copies in build output.
	if requests := server.requests(); len(requests) != 2 {
		t.Errorf("the check made %d requests (%v); it should ask once per distinct address",
			len(requests), requests)
	}
	for _, request := range server.requests() {
		if !strings.HasPrefix(request, "HEAD ") {
			t.Errorf("the check issued %q; design.md specifies a HEAD request", request)
		}
	}
}

// TestCheckNamesTheEntryAndTheAddressThatStoppedResolving is task 3.5's failing
// half, and the shape the specification asks for: the failure names the entry
// and the address. A build log saying "link check failed" sends a reader to
// grep; one saying which address and which device it belonged to does not.
func TestCheckNamesTheEntryAndTheAddressThatStoppedResolving(t *testing.T) {
	devices := fixtureDevices()
	broken := devices[1]

	_, client := serve(t, map[string]int{pathOf(t, broken): http.StatusNotFound})

	ctx, cancel := context.WithTimeout(context.Background(), testTimeout)
	defer cancel()

	failures := Check(ctx, client, devices)

	if len(failures) != 1 {
		t.Fatalf("one address was removed and the check reported %d failures:\n%s",
			len(failures), Report(failures))
	}

	failure := failures[0]
	if failure.Address != broken.Address() {
		t.Errorf("the failure names %q, want %q", failure.Address, broken.Address())
	}
	if !strings.Contains(failure.Reason, "404") {
		t.Errorf("the reason is %q; the server's own status is kept so a rename is distinguishable from a timeout", failure.Reason)
	}

	// Both devices on the broken address are named. A reader who owns either
	// one recognises their board by a different name than the one in the
	// catalogue, and the failure is the only place that would tell them.
	var named []string
	for _, d := range failure.Devices {
		named = append(named, d.Name)
	}
	for _, want := range []string{"Beta Board", "Beta Board, later revision"} {
		if !carries(named, want) {
			t.Errorf("the failure does not name %q; it names %v", want, named)
		}
	}

	message := failure.Error()
	for _, want := range []string{broken.Address(), "404", "Beta Board", "beta", "beta-late"} {
		if !strings.Contains(message, want) {
			t.Errorf("the failure message does not contain %q:\n%s", want, message)
		}
	}

	// The addresses that did resolve are not reported, or the output stops
	// being readable on the build that matters.
	if containsAddress(failures, devices[0].Address()) {
		t.Errorf("the check reported %s, which resolved", devices[0].Address())
	}
}

func containsAddress(failures []Failure, address string) bool {
	for _, f := range failures {
		if f.Address == address {
			return true
		}
	}
	return false
}

// TestCheckReportsEveryAddressThatDoesNotResolve shows the check does not stop
// at the first breakage. A build that reports one and exits tells a reviewer
// to fix one problem and push again to learn about the rest.
func TestCheckReportsEveryAddressThatDoesNotResolve(t *testing.T) {
	devices := fixtureDevices()

	_, client := serve(t, map[string]int{
		pathOf(t, devices[0]): http.StatusNotFound,
		pathOf(t, devices[1]): http.StatusInternalServerError,
	})

	ctx, cancel := context.WithTimeout(context.Background(), testTimeout)
	defer cancel()

	failures := Check(ctx, client, devices)
	if len(failures) != 2 {
		t.Fatalf("two addresses did not resolve and the check reported %d:\n%s", len(failures), Report(failures))
	}

	report := Report(failures)
	for _, want := range []string{"404", "500", devices[0].Address(), devices[1].Address()} {
		if !strings.Contains(report, want) {
			t.Errorf("the report does not mention %q:\n%s", want, report)
		}
	}
}

// TestCheckReportsAnUnreachableServerRatherThanPassing covers the failure that
// a check written only for the 404 case gets wrong. A network that is down,
// a proxy that refuses, or a typo in the host leaves every address
// unreachable; reporting that as a pass would be the worst outcome available,
// because it looks exactly like a working catalogue.
func TestCheckReportsAnUnreachableServerRatherThanPassing(t *testing.T) {
	// Port 1 on the loopback interface: reserved, privileged, and with nothing
	// behind it, so the connection is refused immediately rather than timing
	// out. That keeps the test fast and does not depend on which ports the
	// machine happens to have free.
	target, err := url.Parse("http://127.0.0.1:1")
	if err != nil {
		t.Fatal(err)
	}

	ctx, cancel := context.WithTimeout(context.Background(), testTimeout)
	defer cancel()

	failures := Check(ctx, &http.Client{Transport: redirectTo{target: target}, Timeout: testTimeout}, fixtureDevices())
	if len(failures) != len(addressGroups(fixtureDevices())) {
		t.Fatalf("nothing answered and the check reported %d failures, want %d:\n%s",
			len(failures), len(addressGroups(fixtureDevices())), Report(failures))
	}
	for _, f := range failures {
		if f.Reason == "" {
			t.Errorf("the failure for %s carries no reason; an unreachable server and a missing image must be told apart", f.Address)
		}
	}
}

// TestCheckStopsWhenTheContextIsCancelled shows the check honours cancellation
// rather than running every request out to its timeout after the build has
// already decided it wants to stop.
func TestCheckStopsWhenTheContextIsCancelled(t *testing.T) {
	// A server that answers nothing, so the only thing that can end a request
	// is the caller's context. A check that ignored cancellation would run
	// every address out to its own timeout after the build had decided to stop.
	silent := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		<-r.Context().Done()
	}))
	t.Cleanup(silent.Close)
	target, err := url.Parse(silent.URL)
	if err != nil {
		t.Fatal(err)
	}

	ctx, cancel := context.WithCancel(context.Background())
	cancel()

	done := make(chan []Failure, 1)
	go func() { done <- Check(ctx, &http.Client{Transport: redirectTo{target: target}}, fixtureDevices()) }()

	select {
	case <-done:
	case <-time.After(testTimeout * 4):
		t.Fatal("Check did not return after its context was cancelled")
	}
}
