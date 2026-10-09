package signature

import (
	"bytes"
	"context"
	"fmt"
	"net/http"
	"os"

	"github.com/ProtonMail/go-crypto/openpgp"
	"github.com/ProtonMail/go-crypto/openpgp/armor"
)

// Suffix is what the publisher appends to an image's filename for the detached
// signature that accompanies it. Beside DietPi_RPi5-ARMv8-Trixie.img.xz the
// publisher serves DietPi_RPi5-ARMv8-Trixie.img.xz.asc, which is the naming
// design.md records and the one the address in the catalogue sits beside.
const Suffix = ".asc"

// State is how far a signature verification got. The three are not a scorecard:
// an image is signed or it is not, and one that could not be checked carries the
// reason it could not be checked so nothing downstream can read it as a pass.
type State int

const (
	// Pending is a verification that was never made. It is the zero value, so a
	// Result nobody filled in claims nothing.
	Pending State = iota

	// Signed means the signature verified against the retrieved key.
	Signed

	// BadSignature means it did not. The key it was checked against and the
	// reason it failed are both kept, because "verification failed" is a dead
	// end and the two of them are the start of a diagnosis.
	BadSignature

	// Unverified means the key or the signature could not be retrieved, so
	// nothing was checked. It is deliberately distinct from BadSignature: an
	// unverified image is not a wrong image.
	Unverified
)

// String names the state, for the record written beside the image and for the
// step log this version's output is built from.
func (s State) String() string {
	switch s {
	case Signed:
		return "signed"
	case BadSignature:
		return "bad signature"
	case Unverified:
		return "unverified"
	default:
		return "pending"
	}
}

// Result is one signature verification: how far it got, which key it was against,
// and why it stopped where it did.
type Result struct {
	// State is how far the verification got.
	State State

	// Key is the signing key that was used. It is set in every state, including
	// the one where nothing could be retrieved, so an unverified result can name
	// the key that was sought and the place the tool looked.
	Key Key

	// Source is the address the detached signature was taken from. It is empty
	// when the run never got as far as asking for one, which is the case when the
	// key could not be retrieved.
	Source string

	// Signer is the key ID that made the signature. It is set only when the
	// signature verified: it is the signing subkey rather than the primary key on
	// the certificate, which is what gpg reports and what the record beside the
	// image has to name.
	Signer string

	// Reason is why the signature did not verify, or why nothing could be
	// checked. It is empty for Signed, where there is a key to read instead.
	Reason string
}

// IsVerified reports whether the signature verified against the retrieved key.
// It is the one question a caller that wants to proceed has to ask, and it is
// false for every state but Signed.
func (r Result) IsVerified() bool { return r.State == Signed }

// Error is what the operator is shown, and is what a bad signature is returned
// as so a run halts with the reason attached rather than continuing.
//
// It is defined for every state rather than only for failures for the reason
// fetch's Checksum.Error is: a caller that checked only for a non-nil error
// would otherwise carry an image that failed its signature forward.
func (r Result) Error() string {
	switch r.State {
	case Signed:
		// The second half of this is the specification's own requirement that
		// where the key comes from is stated where a reader will meet it. It is
		// the difference between a result and a result that has been read as
		// establishing more than it does. The fingerprint of what the keyserver
		// actually served is reported rather than checked, and is named for the
		// same reason: it is what was verified against, and a reader is entitled
		// to see it even though nothing here confirms it is the publisher's.
		return fmt.Sprintf("signed by key %s (%s), retrieved at verification time from the public keyserver at %s "+
			"and identified by key ID rather than by a fingerprint a reader could check independently; "+
			"the key served was %s", r.Signer, r.Key.Identity, r.Key.Source, r.Key.Fingerprint)
	case BadSignature:
		return fmt.Sprintf("the signature published at %s does not verify against the publisher's signing key %s "+
			"(%s), retrieved by key ID %s from %s: %s",
			r.Source, r.Key.Fingerprint, r.Key.Identity, r.Key.ID, r.Key.Source, r.Reason)
	case Unverified:
		if r.Source == "" {
			return fmt.Sprintf("unverified: the publisher's signing key %s could not be retrieved from %s (%s)",
				r.Key.ID, r.Key.Source, r.Reason)
		}
		return fmt.Sprintf("unverified: the signature published at %s could not be checked against the "+
			"publisher's signing key %s (%s)", r.Source, r.Key.ID, r.Reason)
	default:
		return "unverified: the signature was never checked"
	}
}

// Verify checks the detached signature published at signatureSource, supplied in
// detached, over the image at imagePath against the retrieved key.
//
// The image is handed to the verifier as an io.Reader rather than as bytes: it is
// roughly 200 MiB, and an operator's machine is not assumed to have twice that
// spare. internal/deps proves the library verifies through a stream.
//
// VerifyDetachedSignature is used rather than CheckArmoredDetachedSignature
// because it returns the signature and with it the key ID that made it, which is
// the value the record beside the image and the step log both have to name.
func (k *Keyring) Verify(imagePath, signatureSource string, detached []byte) Result {
	r := Result{State: Unverified, Key: k.Key, Source: signatureSource}

	image, err := os.Open(imagePath)
	if err != nil {
		r.Reason = err.Error()
		return r
	}
	defer image.Close()

	// The publisher serves the signature as ASCII armor, and the binary form
	// underneath it is what VerifyDetachedSignature reads. That the two steps
	// compose is part of what internal/deps established before this was written.
	block, err := armor.Decode(bytes.NewReader(detached))
	if err != nil {
		r.Reason = fmt.Sprintf("the detached signature is not ASCII armor: %s", err)
		return r
	}

	sig, _, err := openpgp.VerifyDetachedSignature(k.entities, image, block.Body, nil)
	if err != nil {
		r.State = BadSignature
		r.Reason = err.Error()
		return r
	}
	if sig == nil || sig.IssuerKeyId == nil {
		// A signature that verifies and names nothing is not a result anyone can
		// act on, and it is not one anyone should read as a pass either.
		r.State = BadSignature
		r.Reason = "the signature verified but named no key that made it"
		return r
	}

	r.State = Signed
	r.Signer = fmt.Sprintf("%X", *sig.IssuerKeyId)
	return r
}

// RetrieveSignature fetches the detached signature the publisher serves beside
// the image at imageURL.
//
// A failure here is reported as unverified rather than as a bad signature: a
// signature file that could not be retrieved has said nothing about the image,
// and there is nothing for it to disagree with.
func RetrieveSignature(ctx context.Context, client *http.Client, imageURL string) ([]byte, error) {
	return getBody(ctx, client, imageURL+Suffix)
}

// VerifyDownload is the one call a run makes: it retrieves the publisher's
// signing key by its key ID, retrieves the detached signature that accompanies
// the image, and verifies the image already at imagePath against it.
//
// An empty keyserver means DefaultKeyserver, and a nil client means DefaultClient.
//
// It returns an error only for a signature that did not verify. That is the halt
// the specification asks for and it is how the checksum failure beside it is
// reported: a caller that checks for a non-nil error then stops, so nothing that
// failed is carried into decompression and nothing that failed is presented as a
// usable image.
//
// A key or a signature that could not be retrieved returns no error, because a
// keyserver that is down has said nothing about the image, and an operator has
// not been told theirs is bad. What they have been told is that it is unverified,
// and why.
func VerifyDownload(ctx context.Context, client *http.Client, keyserver, imageURL, imagePath string) (Result, error) {
	if client == nil {
		client = DefaultClient()
	}
	if keyserver == "" {
		keyserver = DefaultKeyserver
	}

	keyring, err := RetrieveKey(ctx, client, keyserver, SigningKeyID)
	if err != nil {
		return Result{
			State:  Unverified,
			Key:    Key{ID: SigningKeyID, Source: lookupAddress(keyserver, SigningKeyID)},
			Reason: err.Error(),
		}, nil
	}
	defer keyring.Close()

	detached, err := RetrieveSignature(ctx, client, imageURL)
	if err != nil {
		return Result{State: Unverified, Key: keyring.Key, Reason: err.Error()}, nil
	}

	result := keyring.Verify(imagePath, imageURL+Suffix, detached)
	if result.State == BadSignature {
		return result, result
	}
	return result, nil
}
