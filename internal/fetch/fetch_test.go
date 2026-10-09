// The tests in this file cover section 4 against a local server rather than the
// publisher's, so that go test ./... neither depends on the network nor fails
// when someone else's server is down.
//
// The publisher is stood up here with the behaviour the specification describes
// — a checksum file published beside the image, a connection that can be cut —
// so each case below is a behaviour the spec asks for rather than a restatement
// of what the code does.
package fetch_test

import (
	"bytes"
	"context"
	"crypto/sha256"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/uncommonraven/pisquared/internal/fetch"
)

// fixtureSize is the default size of the image these tests fetch: larger than
// the transfer buffer, so a complete transfer is a few reads rather than one and
// a code path that ignores buffering or progress reporting shows up.
const fixtureSize = 100_000

// fixture is the bytes this section's tests fetch.
//
// It is generated rather than repeated, because a file whose halves are
// identical would let a seam bug pass unnoticed.
func fixture(size int) []byte {
	image := make([]byte, size)
	seed := []byte("DietPi_RPi5-ARMv8-Trixie")
	sum := sha256.Sum256(seed)
	for i := range image {
		image[i] = byte(i*7+len(seed)) ^ seed[i%len(seed)]
	}
	for i := range sum {
		image[i] ^= sum[i]
	}
	return image
}

// digestOf is the sha256 digest of a fixture, which is the value the publisher
// publishes beside it. It takes no *testing.T because the publisher stands the
// fixture's digest through the same function the test's expectation uses, so a
// typo in the digest would otherwise have to agree with itself.
func digestOf(image []byte) string {
	return fmt.Sprintf("%x", sha256.Sum256(image))
}

// digest is the same value, for a test asserting on it.
func digest(t *testing.T, image []byte) string {
	t.Helper()
	return digestOf(image)
}

// publisher stands in for the publisher's server.
type publisher struct {
	// image is the file served at the image address.
	image []byte

	// wrongDigest, when set, is what the checksum file publishes instead of the
	// image's real digest. It is the mismatch case.
	wrongDigest string

	// checksumMissing makes the checksum file unretrievable.
	checksumMissing bool

	// cutAt truncates the image at that offset and closes the connection, as a
	// link that fails part way through does.
	cutAt int

	// noName serves a checksum file that carries digests for several files and
	// none for this one, as a server serving the wrong file does.
	noName bool

	// refuse answers the image address with a 404, as the publisher does for a
	// file it no longer serves.
	refuse bool
}

// serve starts the publisher and returns the address of its image and a client
// that talks to it.
func (p *publisher) serve(t *testing.T) (imageURL string, client *http.Client) {
	t.Helper()

	server := httptest.NewServer(p)
	t.Cleanup(server.Close)
	return server.URL + "/downloads/images/DietPi_RPi5-ARMv8-Trixie.img.xz", server.Client()
}

func (p *publisher) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	// The checksum file sits beside the image under the publisher's own naming,
	// which is what the Suffix in the package refers to.
	if strings.HasSuffix(r.URL.Path, fetch.Suffix) {
		switch {
		case p.checksumMissing:
			http.Error(w, "gone", http.StatusNotFound)
		case p.wrongDigest != "":
			fmt.Fprintf(w, "%s  DietPi_RPi5-ARMv8-Trixie.img.xz\n", p.wrongDigest)
		case p.noName:
			// A checksum file naming other files, which is what a publisher
			// serving the wrong file beside this one looks like.
			fmt.Fprint(w, strings.Repeat("cd", 32)+"  DietPi_RPi1-ARMv6-Trixie.img.xz\n")
			fmt.Fprint(w, strings.Repeat("ef", 32)+"  DietPi_RPi5-ARMv8-Trixie-Digest.txt\n")
		default:
			fmt.Fprintf(w, "%s  DietPi_RPi5-ARMv8-Trixie.img.xz\n", digestOf(p.image))
		}
		return
	}

	if p.cutAt > 0 {
		w.WriteHeader(http.StatusOK)
		w.Write(p.image[:p.cutAt])
		// Aborting is the server-side version of a link going away: the
		// response was started and never finished.
		panic(http.ErrAbortHandler)
	}
	if p.refuse {
		http.NotFound(w, r)
		return
	}
	w.Header().Set("Content-Length", fmt.Sprint(len(p.image)))
	w.Write(p.image)
}

// workingDir is a fresh output directory for a test.
func workingDir(t *testing.T) (output string) {
	t.Helper()
	return filepath.Join(t.TempDir(), "DietPi.img")
}

// TestDownloadAppearsAtTheOutputPathOnlyWhenItCompletes is task 4.1. The
// transfer streams to a temporary name beside the output and is renamed onto it
// once every byte has arrived, so a completed download is at the output path and
// an interrupted one is nowhere near it.
func TestDownloadAppearsAtTheOutputPathOnlyWhenItCompletes(t *testing.T) {
	image := fixture(fixtureSize)

	t.Run("completed", func(t *testing.T) {
		imageURL, client := (&publisher{image: image}).serve(t)
		out := workingDir(t)

		result, err := fetch.Download(context.Background(), client, imageURL, out, nil)
		if err != nil {
			t.Fatalf("a complete transfer failed: %v", err)
		}
		if result.Path != out {
			t.Errorf("the bytes are at %q, want %q", result.Path, out)
		}
		got, err := os.ReadFile(out)
		if err != nil {
			t.Fatalf("the completed transfer is not at the output path: %v", err)
		}
		if !bytes.Equal(got, image) {
			t.Errorf("the bytes at the output path are not the image that was published")
		}
		if result.Bytes != int64(len(image)) {
			t.Errorf("the result reports %d bytes, want %d", result.Bytes, len(image))
		}
		// The part file is gone once the transfer is done, or the output
		// directory carries two copies of a 200 MiB file forever.
		if _, err := os.Stat(out + fetch.PartSuffix); !os.IsNotExist(err) {
			t.Errorf("the part file %q is still there after a completed transfer", out+fetch.PartSuffix)
		}
	})

	t.Run("interrupted", func(t *testing.T) {
		imageURL, client := (&publisher{image: image, cutAt: 40_000}).serve(t)
		out := workingDir(t)

		if _, err := fetch.Download(context.Background(), client, imageURL, out, nil); err == nil {
			t.Fatal("an interrupted transfer reported success")
		}
		if _, statErr := os.Stat(out); !os.IsNotExist(statErr) {
			t.Errorf("an interrupted transfer left a file at the output path, which is indistinguishable from a finished image")
		}
		// The partial bytes are still there, not at the output path where they
		// could be mistaken for the image.
		part, statErr := os.Stat(out + fetch.PartSuffix)
		if statErr != nil {
			t.Fatalf("the part file is not there either: %v", statErr)
		}
		if part.Size() != 40_000 {
			t.Errorf("the part file carries %d bytes, want the 40000 that arrived before the link went away", part.Size())
		}
	})
}

// TestARetriedTransferStartsOverReachesTheSameBytes is the specification's
// "The download is retried" scenario: where an interrupted download is retried
// it starts over rather than continuing from what was received, and the
// verified result is identical to one that was never interrupted.
//
// Resumption was built here first and removed; Download's comment records the
// decision and the artifacts no longer claim it.
func TestARetriedTransferStartsOverReachesTheSameBytes(t *testing.T) {
	image := fixture(200_000)

	// The interrupted attempt that leaves the partial file behind.
	cutURL, cutClient := (&publisher{image: image, cutAt: 70_000}).serve(t)
	out := workingDir(t)
	if _, err := fetch.Download(context.Background(), cutClient, cutURL, out, nil); err == nil {
		t.Fatal("the first attempt was expected to be cut off")
	}

	// The retry, over the same output path.
	imageURL, client := (&publisher{image: image}).serve(t)
	result, err := fetch.Download(context.Background(), client, imageURL, out, nil)
	if err != nil {
		t.Fatalf("the retry failed: %v", err)
	}
	if !result.Checksum.IsVerified() {
		t.Fatalf("the retry reports %s, not verified:\n%s", result.Checksum.State, result.Checksum.Error())
	}
	if result.Checksum.Observed != digest(t, image) {
		t.Errorf("the retried file hashed to %s, want the published %s", result.Checksum.Observed, digest(t, image))
	}

	got, err := os.ReadFile(out)
	if err != nil {
		t.Fatalf("nothing was left at the output path: %v", err)
	}
	if !bytes.Equal(got, image) {
		t.Errorf("the retry did not reach the published bytes: the leftover partial file was continued rather than discarded")
	}
	if len(got) != len(image) {
		t.Errorf("the retried file is %d bytes, want %d", len(got), len(image))
	}
	if _, statErr := os.Stat(out + fetch.PartSuffix); !os.IsNotExist(statErr) {
		t.Errorf("the part file %q is still there after a completed retry", out+fetch.PartSuffix)
	}
}

// TestAMatchIsReportedVerifiedAndAMismatchIsNot is task 4.2. A match is
// reported as verified; a mismatch fails naming both the published and the
// observed digest and leaves nothing at the output path.
func TestAMatchIsReportedVerifiedAndAMismatchIsNot(t *testing.T) {
	image := fixture(60_000)

	t.Run("matching", func(t *testing.T) {
		imageURL, client := (&publisher{image: image}).serve(t)
		out := workingDir(t)

		result, err := fetch.Download(context.Background(), client, imageURL, out, nil)
		if err != nil {
			t.Fatalf("a matching checksum failed the download: %v", err)
		}
		if !result.Checksum.IsVerified() {
			t.Fatalf("the checksum matched and the download reports %s, not verified:\n%s",
				result.Checksum.State, result.Checksum.Error())
		}
		if result.Checksum.Observed != digest(t, image) {
			t.Errorf("the reported digest is %q, want %s", result.Checksum.Observed, digest(t, image))
		}
		if !strings.Contains(result.Checksum.Error(), digest(t, image)) {
			t.Errorf("a passing verification names the value it passed against:\n%s", result.Checksum.Error())
		}
	})

	t.Run("not matching", func(t *testing.T) {
		wrong := strings.Repeat("ab", 32) // a sha256-shaped digest that is not this file's
		imageURL, client := (&publisher{image: image, wrongDigest: wrong}).serve(t)
		out := workingDir(t)

		result, err := fetch.Download(context.Background(), client, imageURL, out, nil)
		if err == nil {
			t.Fatal("a download whose checksum did not match reported success")
		}
		if result.Checksum.State != fetch.Mismatch {
			t.Errorf("the state is %s, want mismatch", result.Checksum.State)
		}

		// The failure names both values, because a mismatch reported as
		// "verification failed" is a dead end and one reported with the two
		// digests is the start of a diagnosis.
		observed := digest(t, image)
		for _, want := range []string{wrong, observed, fetch.Suffix} {
			if !strings.Contains(err.Error(), want) {
				t.Errorf("the failure does not name %q:\n%s", want, err.Error())
			}
		}

		// Nothing is left at the output path, and nothing is left to build on
		// either: bytes that failed their checksum cannot be made correct by
		// receiving the rest of them.
		if _, statErr := os.Stat(out); !os.IsNotExist(statErr) {
			t.Errorf("a failed download left a file at %q", out)
		}
		if _, statErr := os.Stat(out + fetch.PartSuffix); !os.IsNotExist(statErr) {
			t.Errorf("a download that failed its checksum left %q, which invites a retry onto bytes that cannot verify", out+fetch.PartSuffix)
		}
	})
}

// TestAnUnretrievableChecksumIsUnverifiedRatherThanWrong is task 4.3. A
// published checksum that cannot be obtained is reported as unverified with its
// reason, and is reported neither as verified nor as a mismatch — "nobody could
// tell us the answer" and "the answer was different" are different facts.
func TestAnUnretrievableChecksumIsUnverifiedRatherThanWrong(t *testing.T) {
	image := fixture(30_000)
	imageURL, client := (&publisher{image: image, checksumMissing: true}).serve(t)
	out := workingDir(t)

	result, err := fetch.Download(context.Background(), client, imageURL, out, nil)
	if err != nil {
		t.Fatalf("a download whose checksum file is unavailable failed outright, rather than reporting unverified: %v", err)
	}

	if result.Checksum.State != fetch.Unverified {
		t.Fatalf("the state is %s, want unverified", result.Checksum.State)
	}
	if result.Checksum.IsVerified() {
		t.Error("an unverified download reports itself as verified")
	}
	if result.Checksum.Reason == "" {
		t.Error("unverified carries no reason, so nothing can be said about why")
	}
	if !strings.Contains(result.Checksum.Error(), "unverified") {
		t.Errorf("the report does not say the download is unverified:\n%s", result.Checksum.Error())
	}

	// The file is still produced. The specification permits a download that is
	// explicitly reported as unverified, and the report is what distinguishes
	// this from a pass.
	got, err := os.ReadFile(out)
	if err != nil {
		t.Fatalf("an explicitly unverified download left nothing at the output path: %v", err)
	}
	if !bytes.Equal(got, image) {
		t.Errorf("the produced file is not the published image")
	}
}

// TestAPublishedChecksumThatCannotBeReadIsUnverifiedRatherThanWrong covers the
// other half of task 4.3: a checksum file that is reached but says nothing
// usable. It is the same failure to the operator as one that is missing, and it
// must not become a mismatch — there is no published value to disagree with.
func TestAPublishedChecksumThatCannotBeReadIsUnverifiedRatherThanWrong(t *testing.T) {
	image := fixture(20_000)
	imageURL, client := (&publisher{image: image, noName: true}).serve(t)
	out := workingDir(t)

	// A checksum file carrying digests for several files and none naming this
	// one, which is a server serving the wrong file rather than a mismatch.
	result, err := fetch.Download(context.Background(), client, imageURL, out, nil)
	if err != nil {
		t.Fatalf("the download failed rather than reporting unverified: %v", err)
	}
	if result.Checksum.State != fetch.Unverified {
		t.Fatalf("the state is %s, want unverified", result.Checksum.State)
	}
	if result.Checksum.State == fetch.Mismatch {
		t.Error("a checksum file with no digest for this file reports as a mismatch")
	}
	if result.Checksum.Reason == "" {
		t.Error("unverified carries no reason")
	}
}

// TestACancelledTransferLeavesNothingUsable covers the cancellation half of the
// specification's incomplete-download requirement, which section 8 makes
// reachable from the interface.
func TestACancelledTransferLeavesNothingUsable(t *testing.T) {
	image := fixture(200_000)
	imageURL, client := (&publisher{image: image}).serve(t)
	out := workingDir(t)

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	var written int64
	onProgress := func(p fetch.Progress) {
		if p.Written > written {
			written = p.Written
		}
		// Cancelling once well underway, rather than before the transfer starts
		// and rather than racing a deadline that would make this test fail on a
		// slow machine.
		if p.Written > 60_000 {
			cancel()
		}
	}

	if _, err := fetch.Download(ctx, client, imageURL, out, onProgress); !errors.Is(err, context.Canceled) {
		t.Fatalf("a cancelled transfer returned %v, want the cancellation to be reported as one", err)
	}
	if _, statErr := os.Stat(out); !os.IsNotExist(statErr) {
		t.Error("a cancelled transfer left a file at the output path")
	}
	part, statErr := os.Stat(out + fetch.PartSuffix)
	if statErr != nil {
		t.Fatalf("the part file is not there, so a cancelled transfer leaves nothing at all: %v", statErr)
	}
	if part.Size() != written {
		t.Errorf("the part file is %d bytes and the last progress said %d", part.Size(), written)
	}
}

// TestProgressReportsWhatThePublisherDeclared covers the reporting a slow link
// depends on: a total once one is known, and a count that tracks the bytes as
// they arrive.
func TestProgressReportsWhatThePublisherDeclared(t *testing.T) {
	image := fixture(200_000)
	imageURL, client := (&publisher{image: image}).serve(t)
	out := workingDir(t)

	var last fetch.Progress
	events := 0
	if _, err := fetch.Download(context.Background(), client, imageURL, out, func(p fetch.Progress) {
		last, events = p, events+1
	}); err != nil {
		t.Fatalf("the download failed: %v", err)
	}
	if last.Total != int64(len(image)) {
		t.Errorf("the last progress reported a total of %d, want the %d the publisher declared", last.Total, len(image))
	}
	if last.Written != int64(len(image)) {
		t.Errorf("the last progress reported %d bytes written, want %d", last.Written, len(image))
	}
	if events < 2 {
		t.Errorf("progress was reported %d times, so a slow link has nothing to show", events)
	}
}

// TestAServerThatCannotBeReachedIsNotAPass covers the failure a downloader
// written only for the 404 case gets wrong: a network that is down, a proxy
// that refuses, or a typo in the host all leave nothing at the output path, and
// none of them may be reported as verified.
func TestAServerThatCannotBeReachedIsNotAPass(t *testing.T) {
	// Port 1 on the loopback interface: reserved, privileged, and with nothing
	// behind it, so the connection is refused immediately rather than timing
	// out. That keeps the test fast and does not depend on which ports the
	// machine happens to have free.
	out := workingDir(t)

	result, err := fetch.Download(context.Background(),
		&http.Client{}, "http://127.0.0.1:1/downloads/images/DietPi.img.xz", out, nil)
	if err == nil {
		t.Fatal("an unreachable server was reported as a successful download")
	}
	if result.Checksum.IsVerified() {
		t.Error("an unreachable server is reported as verified")
	}
	if _, statErr := os.Stat(out); !os.IsNotExist(statErr) {
		t.Error("a failed transfer left a file at the output path")
	}
}

// TestAServerThatRefusesTheFileIsNotAPass covers the status the publisher can
// answer with other than the bytes, which is what a renamed or removed image
// looks like from the downloader's side.
func TestAServerThatRefusesTheFileIsNotAPass(t *testing.T) {
	image := fixture(10_000)
	refusing := &publisher{image: image, cutAt: 0}
	// cutAt is only the interruption; the refused status is served by the
	// handler above for anything that is not the checksum file, so the refusal
	// is reached by asking for an address the test server answers with a status.
	refusing.refuse = true
	imageURL, client := refusing.serve(t)
	out := workingDir(t)

	result, err := fetch.Download(context.Background(), client, imageURL, out, nil)
	if err == nil {
		t.Fatal("a 404 from the publisher was reported as a successful download")
	}
	if result.Checksum.IsVerified() {
		t.Error("a 404 from the publisher is reported as verified")
	}
	if !strings.Contains(err.Error(), "404") {
		t.Errorf("the failure does not name the status the publisher answered with:\n%s", err.Error())
	}
	if _, statErr := os.Stat(out); !os.IsNotExist(statErr) {
		t.Error("a failed transfer left a file at the output path")
	}
}
