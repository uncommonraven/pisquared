// The tests in this file cover section 5 against a local keyserver and a local
// publisher rather than the real ones, so that go test ./... neither depends on
// somebody else's uptime nor fails when a keyserver is slow.
//
// The publishers stand in here with the behaviour design.md describes — a
// public keyserver that answers a lookup with an armored public key, and a
// detached signature served beside the image — so each case below is a behaviour
// the specification asks for rather than a restatement of what the code does.
package signature_test

import (
	"bytes"
	"context"
	"fmt"
	"io/fs"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"

	"github.com/ProtonMail/go-crypto/openpgp"
	"github.com/ProtonMail/go-crypto/openpgp/armor"
	"github.com/ProtonMail/go-crypto/openpgp/packet"
	"github.com/uncommonraven/pisquared/internal/signature"
)

// signingKey is a throwaway key pair standing in for the publisher's.
//
// It is generated rather than committed because the fixture has to be signed by a
// key whose private half is available to the test and useless to anyone else.
// Only the public half is served, and only to this file's local keyserver.
type signingKey struct {
	entity      *openpgp.Entity
	public      []byte // the armored public key, which is what a keyserver serves
	primary     string // the key ID of the key that actually makes the signature
	fingerprint string // its primary key's fingerprint, the wider identifier
}

// generateSigningKey makes an RSA key pair and its armored public half.
func generateSigningKey(t *testing.T) *signingKey {
	t.Helper()

	entity, err := openpgp.NewEntity(
		"pisquared fixture",
		"a key generated only to sign this package's test fixture",
		"fixture@example.invalid",
		&packet.Config{RSABits: 2048},
	)
	if err != nil {
		t.Fatalf("generating the fixture key: %v", err)
	}

	var public bytes.Buffer
	armored, err := armor.Encode(&public, openpgp.PublicKeyType, nil)
	if err != nil {
		t.Fatalf("armoring the fixture public key: %v", err)
	}
	if err := entity.Serialize(armored); err != nil {
		t.Fatalf("serialising the fixture public key: %v", err)
	}
	if err := armored.Close(); err != nil {
		t.Fatalf("closing the fixture public key's armor: %v", err)
	}

	return &signingKey{
		entity:      entity,
		public:      public.Bytes(),
		primary:     fmt.Sprintf("%X", entity.PrimaryKey.KeyId),
		fingerprint: fmt.Sprintf("%X", entity.PrimaryKey.Fingerprint),
	}
}

// detachedSignature signs image the way the publisher signs its own, so the
// fixture is the shape of the thing being verified.
func (k *signingKey) detachedSignature(t *testing.T, image []byte) []byte {
	t.Helper()

	var sig bytes.Buffer
	if err := openpgp.ArmoredDetachSign(&sig, k.entity, bytes.NewReader(image), nil); err != nil {
		t.Fatalf("signing the fixture image: %v", err)
	}
	return sig.Bytes()
}

// image is the fixture this section's tests verify. It is larger than any buffer
// the verification path would use, so a verifier that read a fixed amount rather
// than the file shows up.
const imageSize = 100_000

func fixtureImage() []byte {
	image := make([]byte, imageSize)
	for i := range image {
		image[i] = byte(i*13 + 7)
	}
	return image
}

// writtenImage puts the fixture on disk and returns its path, because Verify
// opens a path rather than being handed bytes.
func writtenImage(t *testing.T, image []byte) string {
	t.Helper()

	path := filepath.Join(t.TempDir(), "DietPi_RPi5-ARMv8-Trixie.img.xz")
	if err := os.WriteFile(path, image, 0o644); err != nil {
		t.Fatalf("writing the fixture image: %v", err)
	}
	return path
}

// keyserver stands in for the public keyserver the publisher's signing key is
// retrieved from.
type keyserver struct {
	// public is the armored key it answers a lookup with.
	public []byte

	// missing answers the lookup with a 404, as a keyserver does for a key ID it
	// does not carry.
	missing bool

	// asked records the lookup it was last given, so a test can assert the key
	// was asked for by the key ID the tool claims it uses.
	asked url.Values
}

func (k *keyserver) serve(t *testing.T) string {
	t.Helper()

	server := httptest.NewServer(k)
	t.Cleanup(server.Close)
	return server.URL
}

func (k *keyserver) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	k.asked = r.URL.Query()
	if k.missing {
		http.Error(w, "No results found", http.StatusNotFound)
		return
	}
	w.Header().Set("Content-Type", "application/pgp-keys")
	w.Write(k.public)
}

// publisher stands in for the publisher's image server.
type publisher struct {
	// image is what it serves at the image address.
	image []byte

	// detached is the detached signature it serves beside the image.
	detached []byte

	// signatureMissing refuses the signature address, which is what a publisher
	// that stopped signing looks like from the tool's side.
	signatureMissing bool
}

func (p *publisher) serve(t *testing.T) (imageURL string) {
	t.Helper()

	server := httptest.NewServer(p)
	t.Cleanup(server.Close)
	return server.URL + "/downloads/images/DietPi_RPi5-ARMv8-Trixie.img.xz"
}

func (p *publisher) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	if strings.HasSuffix(r.URL.Path, signature.Suffix) {
		if p.signatureMissing {
			http.Error(w, "gone", http.StatusNotFound)
			return
		}
		w.Header().Set("Content-Type", "text/plain")
		w.Write(p.detached)
		return
	}
	w.Header().Set("Content-Length", fmt.Sprint(len(p.image)))
	w.Write(p.image)
}

// TestTheKeyAndTheSignatureAreBothObtained is task 5.1. The publisher's signing
// key is retrieved by its key ID and the detached signature that accompanies the
// image is fetched, and the key material is staged somewhere other than the
// operator's keyring.
func TestTheKeyAndTheSignatureAreBothObtained(t *testing.T) {
	key := generateSigningKey(t)
	image := fixtureImage()
	detached := key.detachedSignature(t, image)

	keyserverURL := (&keyserver{public: key.public}).serve(t)

	keyring, err := signature.RetrieveKey(context.Background(), nil, keyserverURL, key.primary)
	if err != nil {
		t.Fatalf("the signing key could not be retrieved: %v", err)
	}
	defer keyring.Close()

	if keyring.Key.ID != key.primary {
		t.Errorf("the keyring reports it retrieved key %q, want %q", keyring.Key.ID, key.primary)
	}
	if keyring.Key.Source == "" {
		t.Error("the keyring reports no source, so nothing can say where the key came from")
	}
	if !strings.Contains(keyring.Key.Identity, "pisquared fixture") {
		t.Errorf("the keyring reports the key as %q, which does not name the fixture", keyring.Key.Identity)
	}
	if keyring.Key.Fingerprint == "" {
		t.Error("the keyring reports no fingerprint, so the key cannot be named beyond its key ID")
	}

	// The key is staged in a temporary directory rather than in the operator's
	// keyring, and the staged file is the one that was parsed.
	if keyring.Path == "" {
		t.Fatal("the keyring staged the key material nowhere")
	}
	if _, err := os.Stat(keyring.Path); err != nil {
		t.Errorf("the staged key file %s is not there: %v", keyring.Path, err)
	}
	staged, err := os.ReadFile(keyring.Path)
	if err != nil {
		t.Fatalf("reading the staged key file: %v", err)
	}
	if !bytes.Equal(staged, key.public) {
		t.Error("the staged key file is not the key the keyserver served")
	}

	detachedURL := (&publisher{image: image, detached: detached}).serve(t)
	got, err := signature.RetrieveSignature(context.Background(), nil, detachedURL)
	if err != nil {
		t.Fatalf("the detached signature could not be retrieved: %v", err)
	}
	if !bytes.Equal(got, detached) {
		t.Error("the retrieved signature is not the one the publisher served")
	}
}

// TestTheKeyIsRetrievedByKeyIDNamesAPlaceOutsideTheOperatorsKeyring is the other
// half of task 5.1: nothing is written to the operator's own keyring.
//
// The keyring directories a tool that used gpg would write to are pointed at an
// empty directory and left alone. The assertion is a before-and-after comparison
// of that directory, which is the whole of what "running the tool leaves nothing
// behind on their machine" means and is stronger than an assertion that nothing
// was written to a directory the test happened to pick.
func TestTheKeyIsRetrievedByKeyIDNamesAPlaceOutsideTheOperatorsKeyring(t *testing.T) {
	key := generateSigningKey(t)
	image := fixtureImage()
	detached := key.detachedSignature(t, image)

	keyringHome := t.TempDir()
	t.Setenv("HOME", keyringHome)
	t.Setenv("USERPROFILE", keyringHome) // Windows reads its home directory here.
	t.Setenv("GNUPGHOME", keyringHome)

	keyserverURL := (&keyserver{public: key.public}).serve(t)
	imageURL := (&publisher{image: image, detached: detached}).serve(t)

	before := snapshot(t, keyringHome)

	imagePath := writtenImage(t, image)
	if _, err := signature.VerifyDownload(context.Background(), nil, keyserverURL, imageURL, imagePath); err != nil {
		t.Fatalf("a verifiable download failed to verify: %v", err)
	}

	after := snapshot(t, keyringHome)
	if strings.Join(before, "\n") != strings.Join(after, "\n") {
		t.Errorf("the operator's keyring directory changed:\nbefore: %v\nafter:  %v", before, after)
	}
}

// TestTheKeyIsAskedForByTheKeyIDTheToolNames covers the identification the
// specification requires the tool to state: the key is retrieved by key ID, and
// that is the identifier in the request rather than one substituted on the way.
func TestTheKeyIsAskedForByTheKeyIDTheToolNames(t *testing.T) {
	key := generateSigningKey(t)
	server := &keyserver{public: key.public}

	if _, err := signature.RetrieveKey(context.Background(), nil, server.serve(t), key.primary); err != nil {
		t.Fatalf("the signing key could not be retrieved: %v", err)
	}
	if got, want := server.asked.Get("search"), "0x"+key.primary; got != want {
		t.Errorf("the keyserver was asked for %q, want the key ID %q", got, want)
	}
	if got := server.asked.Get("op"); got != "get" {
		t.Errorf("the keyserver was asked for operation %q, want %q", got, "get")
	}
}

// TestAGoodSignatureReportsTheKeyThatMadeIt is task 5.2's first half. A good
// signature reports the key that made it, and says where that key came from.
func TestAGoodSignatureReportsTheKeyThatMadeIt(t *testing.T) {
	key := generateSigningKey(t)
	image := fixtureImage()
	detached := key.detachedSignature(t, image)

	keyserverURL := (&keyserver{public: key.public}).serve(t)
	imageURL := (&publisher{image: image, detached: detached}).serve(t)
	imagePath := writtenImage(t, image)

	keyring, err := signature.RetrieveKey(context.Background(), nil, keyserverURL, key.primary)
	if err != nil {
		t.Fatalf("the signing key could not be retrieved: %v", err)
	}
	defer keyring.Close()

	result := keyring.Verify(imagePath, imageURL+signature.Suffix, detached)
	if !result.IsVerified() {
		t.Fatalf("the download reports %s, not signed:\n%s", result.State, result.Error())
	}
	if result.State != signature.Signed {
		t.Errorf("the state is %s, want signed", result.State)
	}

	// The key that made the signature is named, and it is the one read out of the
	// signature rather than the one the key was looked up by. The real publisher
	// signs with the subkey the tool looks up, so the two agree in production;
	// here they are deliberately different, so a result that echoed the lookup ID
	// back cannot pass.
	if result.Signer != key.primary {
		t.Errorf("the signature is reported as made by key %q, want %q", result.Signer, key.primary)
	}

	// The result states where the key came from, so it is not read as
	// establishing more than it does.
	shown := result.Error()
	for _, want := range []string{key.primary, keyserverURL, "key ID", "fingerprint"} {
		if !strings.Contains(shown, want) {
			t.Errorf("the passing result does not mention %q:\n%s", want, shown)
		}
	}
}

// TestATamperedImageFailsAndNothingIsPresentedAsUsable is task 5.2's other half.
// A tampered image fails, and neither failing case is presented as usable.
func TestATamperedImageFailsAndNothingIsPresentedAsUsable(t *testing.T) {
	key := generateSigningKey(t)
	image := fixtureImage()
	detached := key.detachedSignature(t, image)

	keyserverURL := (&keyserver{public: key.public}).serve(t)
	imageURL := (&publisher{image: image, detached: detached}).serve(t)

	imagePath := writtenImage(t, image)
	tampered := append([]byte(nil), image...)
	tampered[len(tampered)/2] ^= 0x01
	if err := os.WriteFile(imagePath, tampered, 0o644); err != nil {
		t.Fatalf("tampering with the image: %v", err)
	}

	keyring, err := signature.RetrieveKey(context.Background(), nil, keyserverURL, key.primary)
	if err != nil {
		t.Fatalf("the signing key could not be retrieved: %v", err)
	}
	defer keyring.Close()

	result := keyring.Verify(imagePath, imageURL+signature.Suffix, detached)
	if result.State != signature.BadSignature {
		t.Fatalf("a tampered image reports %s, want a bad signature:\n%s", result.State, result.Error())
	}
	if result.IsVerified() {
		t.Error("an image that failed its signature reports itself as signed")
	}
	if result.Reason == "" {
		t.Error("a bad signature carries no reason, so nothing can be said about why")
	}
}

// TestAnUnretrievableKeyIsUnverifiedRatherThanABadSignature is task 5.3. A
// signing key that cannot be retrieved is reported as unverified with its reason,
// and is reported neither as verified nor as a bad signature — "nobody could tell
// us" and "the answer was no" are different facts.
func TestAnUnretrievableKeyIsUnverifiedRatherThanABadSignature(t *testing.T) {
	key := generateSigningKey(t)
	image := fixtureImage()
	detached := key.detachedSignature(t, image)

	keyserverURL := (&keyserver{public: key.public, missing: true}).serve(t)
	imageURL := (&publisher{image: image, detached: detached}).serve(t)
	imagePath := writtenImage(t, image)

	result, err := signature.VerifyDownload(context.Background(), nil, keyserverURL, imageURL, imagePath)
	if err != nil {
		t.Fatalf("a key that could not be retrieved failed the run outright, rather than reporting unverified: %v", err)
	}

	if result.State != signature.Unverified {
		t.Fatalf("the state is %s, want unverified", result.State)
	}
	if result.IsVerified() {
		t.Error("an unverified image reports itself as signed")
	}
	if result.State == signature.BadSignature {
		t.Error("a key that could not be retrieved reports as a bad signature, which tells the operator their image is bad when nobody said so")
	}
	if result.Reason == "" {
		t.Error("unverified carries no reason, so nothing can be said about why")
	}
	shown := result.Error()
	if strings.Contains(shown, "does not verify") {
		t.Errorf("an unretrievable key is reported as a signature that does not verify:\n%s", shown)
	}
	if !strings.Contains(shown, "unverified") {
		t.Errorf("the report does not say the signature is unverified:\n%s", shown)
	}
	// The key it was looking for and the place it looked are both named, so the
	// operator can go and ask that same server themselves.
	if !strings.Contains(shown, signature.SigningKeyID) {
		t.Errorf("the report does not name the key it could not retrieve:\n%s", shown)
	}
	if !strings.Contains(shown, keyserverURL) {
		t.Errorf("the report does not name the keyserver it asked:\n%s", shown)
	}
}

// TestAnUnretrievableSignatureIsUnverifiedRatherThanABadSignature covers the
// same distinction from the other side: a detached signature that could not be
// fetched has said nothing about the image, so it is not a bad signature either.
func TestAnUnretrievableSignatureIsUnverifiedRatherThanABadSignature(t *testing.T) {
	key := generateSigningKey(t)
	image := fixtureImage()
	detached := key.detachedSignature(t, image)

	keyserverURL := (&keyserver{public: key.public}).serve(t)
	imageURL := (&publisher{image: image, detached: detached, signatureMissing: true}).serve(t)
	imagePath := writtenImage(t, image)

	result, err := signature.VerifyDownload(context.Background(), nil, keyserverURL, imageURL, imagePath)
	if err != nil {
		t.Fatalf("a signature that could not be retrieved failed the run outright, rather than reporting unverified: %v", err)
	}
	if result.State != signature.Unverified {
		t.Fatalf("the state is %s, want unverified", result.State)
	}
	if result.State == signature.BadSignature {
		t.Error("a signature file that could not be fetched reports as a bad signature")
	}
	if result.Reason == "" {
		t.Error("unverified carries no reason")
	}
}

// TestABadSignatureHaltsTheRun is task 5.4. A signature that does not verify is
// the error that halts a run, with the reason shown, which is how the checksum
// failure beside it is reported.
func TestABadSignatureHaltsTheRun(t *testing.T) {
	key := generateSigningKey(t)
	image := fixtureImage()
	detached := key.detachedSignature(t, image)

	keyserverURL := (&keyserver{public: key.public}).serve(t)
	imageURL := (&publisher{image: image, detached: detached}).serve(t)

	imagePath := writtenImage(t, image)
	tampered := append([]byte(nil), image...)
	tampered[0] ^= 0xff
	if err := os.WriteFile(imagePath, tampered, 0o644); err != nil {
		t.Fatalf("tampering with the image: %v", err)
	}

	// This is the run: it verifies, and it stops when the verification fails.
	// Written out rather than asserted on, because the halt is a property of the
	// caller's control flow and not of any single returned value.
	carriedForward := false
	result, err := signature.VerifyDownload(context.Background(), nil, keyserverURL, imageURL, imagePath)
	switch {
	case err != nil:
		// The halt. The reason is shown, not withheld.
		t.Logf("halted: %v", err)
	case result.IsVerified() || result.State == signature.Unverified:
		carriedForward = true
	}

	if err == nil {
		t.Fatal("a signature that did not verify did not halt the run")
	}
	if carriedForward {
		t.Error("the run continued past an image whose signature did not verify")
	}
	if result.State != signature.BadSignature {
		t.Errorf("the state is %s, want a bad signature", result.State)
	}

	shown := err.Error()
	if !strings.Contains(shown, "does not verify") {
		t.Errorf("the halt does not say the signature did not verify:\n%s", shown)
	}
	// The key it was checked against is named by the fingerprint of what the
	// keyserver actually served, so an operator can see whether the key that came
	// back is the one the publisher uses rather than only that the tool asked.
	if !strings.Contains(shown, key.fingerprint) {
		t.Errorf("the halt does not name the key it was checked against:\n%s", shown)
	}
	if !strings.Contains(shown, signature.SigningKeyID) {
		t.Errorf("the halt does not name the key ID the key was retrieved by:\n%s", shown)
	}
	if !strings.Contains(shown, imageURL+signature.Suffix) {
		t.Errorf("the halt does not name the signature it checked:\n%s", shown)
	}
}

// snapshot lists every path under dir, so a test can show a run left a directory
// exactly as it found it.
func snapshot(t *testing.T, dir string) []string {
	t.Helper()

	var found []string
	if err := filepath.WalkDir(dir, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		rel, relErr := filepath.Rel(dir, path)
		if relErr != nil {
			return relErr
		}
		found = append(found, rel)
		return nil
	}); err != nil {
		t.Fatalf("listing %s: %v", dir, err)
	}
	sort.Strings(found)
	return found
}
