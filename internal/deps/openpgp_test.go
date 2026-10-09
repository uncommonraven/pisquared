package deps

import (
	"bytes"
	"errors"
	"io"
	"os"
	"testing"

	"github.com/ProtonMail/go-crypto/openpgp"
	"github.com/ProtonMail/go-crypto/openpgp/armor"
	openpgp_errors "github.com/ProtonMail/go-crypto/openpgp/errors"
	"github.com/ProtonMail/go-crypto/openpgp/packet"
)

// The proof this file records is that github.com/ProtonMail/go-crypto/openpgp
// verifies a detached RSA signature, which is the one thing section 6 of this
// version asks of an OpenPGP library and the reason the library was chosen over
// the frozen one in golang.org/x/crypto.
//
// The choice, since design.md recorded it as open:
//
//   - golang.org/x/crypto/openpgp is deprecated. Its own documentation says the
//     package "is unsafe by design, and has numerous known security issues. It
//     is not maintained, and should not be used", and points at the ProtonMail
//     fork as the maintained drop-in replacement. Choosing a library whose own
//     documentation says not to use it, for the one step in this version that
//     establishes whether bytes came from the publisher, is not a trade-off
//     worth making.
//   - github.com/ProtonMail/go-crypto/openpgp is that fork. It is API-compatible
//     with the frozen package — CheckDetachedSignature, NewEntity, EntityList and
//     the rest carry over — so nothing here is written against a shape the other
//     could not also have, and it additionally offers VerifyDetachedSignature,
//     which returns the signature itself and so the issuing key ID, which
//     sections 6, 8, and 9 all have to report.
//
// Two names differ from the frozen package and are worth knowing before section
// 6 is written rather than after: the check is CheckArmoredDetachedSignature
// where the frozen package called it CheckArmoredDetachSign, and the sentinel
// error is openpgp/errors.ErrUnknownIssuer rather than openpgp.ErrUnknownIssuer.
// Neither is a different capability; both are the same API spelled the way the
// fork spells it.
//
// The verified signature here was made by a key generated at fixture time and
// thrown away; only the public half was kept. See testdata/signature/generate.go.
//
// This is a library proof, not the feature. Section 6 is where the key is
// fetched from a keyserver at run time, held outside the operator's keyring,
// and turned into a result the record can carry. None of that is here.

const (
	fixturePayload   = "testdata/signature/payload.bin"
	fixtureSignature = "testdata/signature/payload.bin.asc"
	fixturePublicKey = "testdata/signature/signer.pub.asc"
)

// readKeyring loads the fixture's public key. Only the public half was kept, so
// this is all a verifier is ever given.
func readKeyring(t *testing.T) openpgp.EntityList {
	t.Helper()
	f, err := os.Open(fixturePublicKey)
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()

	keyring, err := openpgp.ReadArmoredKeyRing(f)
	if err != nil {
		t.Fatalf("reading the fixture public key: %v", err)
	}
	if len(keyring) == 0 {
		t.Fatal("the fixture public key held no entities")
	}
	return keyring
}

func openFixture(t *testing.T, name string) *os.File {
	t.Helper()
	f, err := os.Open(name)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { f.Close() })
	return f
}

// TestOpenPGPVerifiesADetachedRSASignature is the proof: a detached signature
// over a file, checked against a public key, with nothing else involved.
func TestOpenPGPVerifiesADetachedRSASignature(t *testing.T) {
	keyring := readKeyring(t)

	signer, err := openpgp.CheckArmoredDetachedSignature(
		keyring,
		openFixture(t, fixturePayload),
		openFixture(t, fixtureSignature),
		nil,
	)
	if err != nil {
		t.Fatalf("verifying the fixture's detached signature: %v", err)
	}
	if signer == nil {
		t.Fatal("the signature verified but named no signer")
	}

	t.Logf("signed by %v, primary key %X", signer.Identities, signer.PrimaryKey.KeyId)
}

// TestOpenPGPReportsTheKeyThatSignedTheImage is a separate proof because
// sections 6, 8, and 9 all have to name the key rather than say only that
// something verified. A library that returned a bare error would leave those
// sections with nothing to write down.
func TestOpenPGPReportsTheKeyThatSignedTheImage(t *testing.T) {
	keyring := readKeyring(t)

	// The publisher serves its signature as ASCII armor at <image>.img.xz.asc,
	// and VerifyDetachedSignature reads the binary form, so the armor is
	// decoded first. That the two steps compose is part of what section 6 needs.
	block, err := armor.Decode(openFixture(t, fixtureSignature))
	if err != nil {
		t.Fatalf("decoding the armored signature: %v", err)
	}

	sig, signer, err := openpgp.VerifyDetachedSignature(
		keyring,
		openFixture(t, fixturePayload),
		block.Body,
		nil,
	)
	if err != nil {
		t.Fatalf("verifying the fixture's detached signature: %v", err)
	}
	if sig == nil {
		t.Fatal("VerifyDetachedSignature returned no signature")
	}

	// The key ID is the value the publisher's own key is looked up by, and the
	// one design.md says the documentation has to state rather than leaving a
	// reader to assume more than was established.
	keyID := sig.IssuerKeyId
	if keyID == nil {
		t.Fatal("the signature named no issuing key")
	}
	if signer == nil || signer.PrimaryKey == nil {
		t.Fatal("the signature verified but named no signer entity")
	}
	if *keyID == 0 {
		t.Error("the issuing key ID is zero, which identifies nothing")
	}

	t.Logf("signature issued by key %X", *keyID)
}

// TestOpenPGPRejectsATamperedPayload is the other half. A verifier that accepts
// altered bytes is not a verifier, and this is the failure the whole stage
// exists to catch.
func TestOpenPGPRejectsATamperedPayload(t *testing.T) {
	keyring := readKeyring(t)

	payload, err := os.ReadFile(fixturePayload)
	if err != nil {
		t.Fatal(err)
	}
	payload[len(payload)/2] ^= 0x01

	_, err = openpgp.CheckArmoredDetachedSignature(
		keyring,
		bytes.NewReader(payload),
		openFixture(t, fixtureSignature),
		nil,
	)
	if err == nil {
		t.Fatal("a payload with one bit changed verified against the fixture's signature")
	}
	t.Logf("one flipped bit was rejected: %v", err)
}

// TestOpenPGPRejectsASignatureFromAnUnknownKey shows the failure that a
// substituted key would produce, which is the threat the check is for: bytes
// that carry a valid signature by a key the publisher does not use.
func TestOpenPGPRejectsASignatureFromAnUnknownKey(t *testing.T) {
	other := generateEntity(t)

	_, err := openpgp.CheckArmoredDetachedSignature(
		openpgp.EntityList{other},
		openFixture(t, fixturePayload),
		openFixture(t, fixtureSignature),
		nil,
	)
	if err == nil {
		t.Fatal("the publisher's signature verified against a key that is not the publisher's")
	}
	if !errors.Is(err, openpgp_errors.ErrUnknownIssuer) {
		t.Errorf("verifying against an unrelated key returned %v, want it to name the issuer as unknown", err)
	}
	t.Logf("an unrelated key was rejected: %v", err)
}

// generateEntity makes a throwaway RSA key. Used to stand in for a substituted
// key, which is the one thing the real publisher's keyring must never accept.
func generateEntity(t *testing.T) *openpgp.Entity {
	t.Helper()
	entity, err := openpgp.NewEntity(
		"pisquared other",
		"a key generated to stand in for a substituted one",
		"other@example.invalid",
		&packet.Config{RSABits: 2048},
	)
	if err != nil {
		t.Fatal(err)
	}
	return entity
}

// TestOpenPGPVerificationStreamsFromDisk shows that verification does not need
// the file resident, which matters because the file is roughly 200 MiB and the
// operator's machine is not assumed to have twice that spare. The file is handed
// over as an io.Reader and never as a byte slice here; that is the claim.
func TestOpenPGPVerificationStreamsFromDisk(t *testing.T) {
	keyring := readKeyring(t)

	info, err := os.Stat(fixturePayload)
	if err != nil {
		t.Fatal(err)
	}
	if info.Size() == 0 {
		t.Fatal("the fixture payload is empty")
	}

	var buffered int
	counting := &countingReader{r: openFixture(t, fixturePayload), seen: &buffered}

	if _, err := openpgp.CheckArmoredDetachedSignature(
		keyring,
		counting,
		openFixture(t, fixtureSignature),
		nil,
	); err != nil {
		t.Fatalf("verifying through a streaming reader: %v", err)
	}
	if buffered != int(info.Size()) {
		t.Errorf("the verifier read %d bytes of a %d byte file", buffered, info.Size())
	}
	t.Logf("verified %d bytes read from disk as a stream", buffered)
}

type countingReader struct {
	r    io.Reader
	seen *int
}

func (c *countingReader) Read(p []byte) (int, error) {
	n, err := c.r.Read(p)
	*c.seen += n
	return n, err
}
