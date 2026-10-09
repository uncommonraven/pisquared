package signature

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/ProtonMail/go-crypto/openpgp"
)

// SigningKeyID is the publisher's signing key, identified the way a keyserver
// identifies a key: by its 64-bit key ID.
//
// The publisher signs images with a signing subkey of an RSA 4096 primary key
// created 2022-08-17, and the subkey that makes the signatures is
// 536B2A4A2E72D870.
//
// The identifier is a key ID and not a fingerprint, which is the trade-off
// design.md records as accepted: a reader cannot check 64 bits against a value
// they were given out of band, and an attacker able to answer for the keyserver
// can substitute a key. It is also what makes a rotation a non-event here — the
// key is fetched at verification time rather than pinned — and it is why the
// result names where the key came from and how it was identified, so a passing
// verification is not read as establishing more than it does.
const SigningKeyID = "536B2A4A2E72D870"

// DefaultKeyserver is the public keyserver the publisher's signing key is
// retrieved from, and the one the publisher's own imaging script names.
//
// The transport is HTTPS rather than hkp's plaintext. design.md records "hkp://
// is plaintext and unauthenticated" as a cost of the adopted approach, and here
// it is a cost with nothing to offset it: the key is the one input to this
// verification an attacker would most want to replace, and the same keyserver
// serves it over TLS. What remains of the recorded cost is the third-party trust
// and the 64-bit identification, and TLS changes neither.
const DefaultKeyserver = "https://keyserver.ubuntu.com"

// keyTimeout bounds retrieving the signing key. Longer than the download's
// checksum read beside it, because a keyserver is somebody else's server being
// asked to do somebody else's work, and a slow one is not a failed verification.
const keyTimeout = 30 * time.Second

// maxRetrievedSize bounds how much of a retrieved armored file is read. The real
// ones are a 10 kB key and an 833 byte signature; this is here so a server that
// answers with something enormous fails as a size problem rather than as the
// tool's memory being consumed by an unrelated file.
const maxRetrievedSize = 1 << 20

// DefaultClient is the client key retrieval and verification use when handed
// none. It is a function rather than a package variable so a test has no shared
// mutable client to race over.
func DefaultClient() *http.Client {
	return &http.Client{Timeout: keyTimeout}
}

// Key is the publisher's signing key: what it is, and where it came from.
type Key struct {
	// ID is the key ID the key was retrieved by. It is the identifier, and the
	// one the tool's output names, because it is the only part of this a reader
	// could go and check.
	ID string

	// Source is the address the key was retrieved from. It is kept even when
	// nothing was retrieved, so a report of "unverified" can name the place
	// that did not answer.
	Source string

	// Identity is what the key says it is, in the publisher's own words. It is
	// read from the key rather than pinned, so it is reported rather than
	// checked against — the same limit the key-ID identification carries.
	Identity string

	// Fingerprint is the primary key's fingerprint as retrieved, for the same
	// reason and with the same limit.
	Fingerprint string
}

// Keyring is the publisher's signing key, held outside the operator's own.
//
// design.md records why it is not in their keyring: running this tool leaves
// nothing behind on their machine. The key is staged in a temporary directory
// this keyring creates and Close removes, so the keyring the operator uses for
// their own mail is neither consulted nor written to.
type Keyring struct {
	// Key is what was retrieved and from where.
	Key Key

	// Path is the staged key file, inside the temporary directory. It is
	// exported because "where did the key material go" is a question worth
	// being able to answer about a keyring that claims to hold nothing on the
	// operator's machine.
	Path string

	// dir is the temporary directory the key material sits in, removed by Close.
	dir string

	// entities is the parsed key, which is what Verify reads.
	entities openpgp.EntityList
}

// Close removes the temporary directory and the key material in it. It is safe
// to call on a Keyring that failed to retrieve, and to call twice.
func (k *Keyring) Close() error {
	if k == nil {
		return nil
	}
	return os.RemoveAll(k.dir)
}

// RetrieveKey obtains the signing key named by id from keyserver and holds it in
// a temporary directory rather than in the operator's keyring.
//
// An empty keyserver means DefaultKeyserver, and a nil client means
// DefaultClient.
//
// Every failure here is a key that could not be retrieved, which the caller
// reports as unverified rather than as a bad signature: a keyserver that is down
// has said nothing about the image.
func RetrieveKey(ctx context.Context, client *http.Client, keyserver, id string) (*Keyring, error) {
	if strings.TrimSpace(id) == "" {
		return nil, errors.New("no key ID was given to retrieve the publisher's signing key by")
	}
	if keyserver == "" {
		keyserver = DefaultKeyserver
	}
	if client == nil {
		client = DefaultClient()
	}

	source := lookupAddress(keyserver, id)
	armored, err := getBody(ctx, client, source)
	if err != nil {
		return nil, err
	}

	dir, err := os.MkdirTemp("", "pisquared-signing-key-")
	if err != nil {
		return nil, fmt.Errorf("cannot create a temporary directory for the signing key: %w", err)
	}
	path := filepath.Join(dir, "signer.asc")
	if err := os.WriteFile(path, armored, 0o600); err != nil {
		os.RemoveAll(dir)
		return nil, fmt.Errorf("cannot stage the signing key at %s: %w", path, err)
	}

	// The key is parsed from the staged file rather than from the bytes that
	// arrived, so the path that reads it is the path that would read a key an
	// operator already had, and so the file that was consulted is a real file.
	entities, err := readKeyring(path)
	if err != nil {
		os.RemoveAll(dir)
		return nil, fmt.Errorf("the key served at %s is not a usable OpenPGP key: %w", source, err)
	}

	return &Keyring{
		Key: Key{
			ID:          id,
			Source:      source,
			Identity:    identityOf(entities),
			Fingerprint: fingerprintOf(entities),
		},
		Path:     path,
		dir:      dir,
		entities: entities,
	}, nil
}

// lookupAddress is the address a key ID is asked for at.
//
// It is the keyserver's own lookup endpoint, which is what "following the
// publisher's own documented approach" means here: the publisher's imaging
// script asks a keyserver for this key by ID, and this is that request.
func lookupAddress(keyserver, id string) string {
	address := strings.TrimSuffix(keyserver, "/") + "/pks/lookup"
	query := url.Values{}
	query.Set("op", "get")
	query.Set("search", "0x"+strings.TrimPrefix(strings.TrimSpace(id), "0x"))
	return address + "?" + query.Encode()
}

// getBody retrieves a file over HTTP and reads it whole.
//
// A key and a signature are kilobytes, so reading them whole costs nothing and
// saves a streaming path nothing needs. The body is bounded for the reason
// fetch's checksum read is bounded.
//
// A nil client means http.DefaultClient, which is what the retrieval beside it
// in fetch does. A caller that wants the key timeout hands this DefaultClient's
// replacement in.
func getBody(ctx context.Context, client *http.Client, address string) ([]byte, error) {
	if client == nil {
		client = http.DefaultClient
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, address, nil)
	if err != nil {
		return nil, fmt.Errorf("cannot ask for what is published at %s: %w", address, err)
	}
	resp, err := client.Do(req)
	if err != nil {
		return nil, fmt.Errorf("%s could not be retrieved: %w", address, err)
	}
	// Closing the body is what releases the connection. A retrieval that leaked
	// one per attempt would exhaust the pool rather than fail visibly.
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("%s could not be retrieved: the server answered %s", address, resp.Status)
	}
	body, err := io.ReadAll(io.LimitReader(resp.Body, maxRetrievedSize))
	if err != nil {
		return nil, fmt.Errorf("%s could not be read: %w", address, err)
	}
	return body, nil
}

// readKeyring parses an armored OpenPGP key file. It is the whole of what turns
// what a keyserver served into something that can verify a signature.
func readKeyring(path string) (openpgp.EntityList, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	return openpgp.ReadArmoredKeyRing(f)
}

// identityOf is what the key says it is. The first identity is taken, because a
// certificate carries one identity here; an empty string is the honest answer
// when a key carries none.
func identityOf(entities openpgp.EntityList) string {
	for _, entity := range entities {
		for name := range entity.Identities {
			return name
		}
	}
	return ""
}

// fingerprintOf is the primary key's fingerprint as retrieved.
func fingerprintOf(entities openpgp.EntityList) string {
	for _, entity := range entities {
		if entity.PrimaryKey == nil {
			continue
		}
		return fmt.Sprintf("%X", entity.PrimaryKey.Fingerprint)
	}
	return ""
}
