package fetch

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"path"
	"regexp"
	"strings"
)

// Algorithm is the digest this tool computes and the publisher publishes.
//
// It is named rather than inferred: the tool verifies against the publisher's
// value, and a reader is entitled to know which digest makes two values
// comparable.
const Algorithm = "sha256"

// Suffix is what the publisher appends to an image's filename for its published
// digest. Beside DietPi_RPi5-ARMv8-Trixie.img.xz the publisher serves
// DietPi_RPi5-ARMv8-Trixie.img.xz.sha256, which is the naming design.md records
// and the one the address in the catalogue sits beside.
const Suffix = ".sha256"

// maxPublishedSize bounds how much of a published checksum file is read. The
// real ones are one line per image; this is here so a server that answers the
// request with something enormous fails as a parse problem rather than as the
// tool's memory being consumed by an unrelated file.
const maxPublishedSize = 1 << 20

// State is how far a verification got. The three are not a scorecard: a
// download is verified or it is not, and an unverified download carries the
// reason it is unverified so nothing downstream can read it as a pass.
type State int

const (
	// Pending is a published value that has been retrieved and not yet compared
	// against anything. It is never returned by Download; it exists so Retrieve
	// has a state to return before a file exists to hash.
	Pending State = iota

	// Verified means the computed digest equalled the published one.
	Verified

	// Mismatch means it did not. The published and observed values are both
	// kept, because a mismatch reported as "verification failed" is a dead end
	// and one reported with both values is the start of a diagnosis.
	Mismatch

	// Unverified means the published value could not be retrieved, or could not
	// be read out of what was retrieved. It is deliberately distinct from
	// Mismatch: an unverified download is not a wrong download.
	Unverified
)

// String names the state, for the record written beside the image and for the
// step log this version's output is built from.
func (s State) String() string {
	switch s {
	case Verified:
		return "verified"
	case Mismatch:
		return "mismatch"
	case Unverified:
		return "unverified"
	default:
		return "pending"
	}
}

// Checksum is one verification: what was published, what was observed, and how
// far the comparison got.
type Checksum struct {
	// State is how far the verification got. Download never leaves it Pending.
	State State

	// Source is the address the published value was taken from. It is kept even
	// when nothing was retrieved, so a report of "unverified" can name the place
	// that did not answer.
	Source string

	// Expected is the published digest, empty when none could be retrieved.
	Expected string

	// Observed is the digest computed from the bytes that arrived.
	Observed string

	// Reason is why the published value could not be retrieved or read. It is
	// empty for Verified and Mismatch, where there is a value to read instead.
	Reason string
}

// IsVerified reports whether the published value was retrieved and matched. It
// is the one question a caller that wants to proceed has to ask, and it is
// false for every state but Verified.
func (c Checksum) IsVerified() bool { return c.State == Verified }

// Error is what the operator is shown, and is what a mismatch is returned as so
// that a run halts with the reason attached rather than continuing.
//
// It is defined for every state rather than only for failures because a
// mismatch has to be the error that stops the run: a caller that checked only
// for a non-nil error would otherwise carry an image that failed its checksum
// into decompression.
func (c Checksum) Error() string {
	switch c.State {
	case Verified:
		return fmt.Sprintf("verified against %s: %s", c.Source, c.Expected)
	case Mismatch:
		return fmt.Sprintf("%s does not match the checksum published at %s: expected %s, observed %s",
			DescribeSource(c.Source), c.Source, c.Expected, c.Observed)
	case Unverified:
		return fmt.Sprintf("unverified: the published %s checksum at %s could not be read (%s)",
			Algorithm, c.Source, c.Reason)
	default:
		return fmt.Sprintf("unverified: %s was retrieved and nothing was compared against it", c.Source)
	}
}

// DescribeSource turns a checksum address back into the filename it describes,
// so a message reads as a statement about a file rather than about a URL.
func DescribeSource(source string) string {
	if i := strings.LastIndex(source, "/"); i >= 0 && i+1 < len(source) {
		return source[i+1:]
	}
	return source
}

// Compare records the digest computed from the bytes that arrived and returns
// the verification in its final state.
//
// An unverified or pending value stays as it is rather than becoming a
// mismatch. Comparing against a value nobody published produces a difference,
// and reporting that difference as a checksum failure would be a false claim
// about bytes that were never contradicted by anything.
func (c Checksum) Compare(observed string) Checksum {
	if c.State != Pending {
		return c
	}
	c.Observed = strings.ToLower(observed)
	switch {
	case c.Expected == "":
		c.State = Unverified
		c.Reason = "the published value carried no digest for this file"
	case c.Expected == c.Observed:
		c.State = Verified
	default:
		c.State = Mismatch
	}
	return c
}

// Retrieve fetches the publisher's published digest for the file at imageURL.
//
// The three outcomes are distinct and all three are returned rather than one
// being turned into another: a published value that was retrieved, one that
// could not be fetched, and one that was fetched but could not be read out of
// it. A transport failure and a 404 are reported with the reason they happened
// and neither is reported as a mismatch.
//
// A non-nil error is returned only for a caller-side problem — an unusable
// client, say. Everything about the publisher's side of the exchange is
// reported through the returned Checksum, because it is a fact about the run
// rather than a fault in the call.
func Retrieve(ctx context.Context, client *http.Client, imageURL string) (Checksum, error) {
	source := imageURL + Suffix
	c := Checksum{State: Unverified, Source: source}

	if client == nil {
		client = http.DefaultClient
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, source, nil)
	if err != nil {
		return c, fmt.Errorf("cannot ask for the published checksum at %s: %w", source, err)
	}

	resp, err := client.Do(req)
	if err != nil {
		c.Reason = err.Error()
		return c, nil
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		c.Reason = fmt.Sprintf("the server answered %s", resp.Status)
		return c, nil
	}

	body, err := io.ReadAll(io.LimitReader(resp.Body, maxPublishedSize))
	if err != nil {
		c.Reason = err.Error()
		return c, nil
	}

	want, err := ParsePublished(string(body), Filename(imageURL))
	if err != nil {
		c.Reason = err.Error()
		return c, nil
	}
	c.State = Pending
	c.Expected = want
	return c, nil
}

// digestLine finds a hex digest and whatever follows it on one line.
var digestLine = regexp.MustCompile(`\b([0-9a-fA-F]{64})\b`)

// Filename is the publisher's filename for the file at an address, which is
// what its checksum file labels the digest with.
//
// The path is parsed rather than sliced, because an address can carry a query
// string and a filename extracted from one must not carry the query with it.
func Filename(address string) string {
	if parsed, err := url.Parse(address); err == nil && parsed.Path != "" {
		return path.Base(parsed.Path)
	}
	return path.Base(address)
}

// ParsePublished reads a publisher's checksum file and returns the digest it
// publishes for filename.
//
// Three shapes are accepted, because a file in sha256sum's format and a file
// holding one bare digest are both things a publisher serves and neither is
// documented upstream:
//
//   - one digest and the filename beside it, sha256sum's own format, with or
//     without its binary-mode asterisk,
//   - the same, among digests for several files, where only the line naming
//     this file is used,
//   - a single bare digest with nothing beside it.
//
// A file carrying several digests and none naming the requested file is a
// refusal rather than a guess. Returning the first line's digest would verify
// this image against some other file's value, which is a passing test of
// nothing.
func ParsePublished(body, filename string) (string, error) {
	var digests []string
	for _, line := range strings.Split(body, "\n") {
		at := digestLine.FindStringSubmatchIndex(line)
		if at == nil {
			continue
		}
		digest := strings.ToLower(line[at[2]:at[3]])
		digests = append(digests, digest)

		// sha256sum separates the digest from the name by two spaces and marks
		// binary mode with a leading asterisk, so everything between the end of
		// the digest and the first remaining field is decoration.
		rest := strings.Fields(strings.TrimLeft(line[at[3]:], " \t*"))
		if len(rest) > 0 && rest[0] == filename {
			return digest, nil
		}
	}

	switch len(digests) {
	case 0:
		return "", fmt.Errorf("it carries no %s digest", Algorithm)
	case 1:
		return digests[0], nil
	default:
		return "", fmt.Errorf("it carries %d digests and none is labelled %s", len(digests), filename)
	}
}
