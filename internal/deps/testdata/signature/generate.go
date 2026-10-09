// Command generate writes the signature fixture that internal/deps proves
// github.com/ProtonMail/go-crypto/openpgp with.
//
// It generates an RSA key pair, signs a payload with it, and writes the
// payload, the detached signature, and the public key. Only the public key is
// committed. The private key is discarded here and is not needed to verify
// anything, which is the point: the fixture is verifiable without it.
//
// Regenerate with:
//
//	go run ./internal/deps/testdata/signature/generate.go
package main

import (
	"bytes"
	"crypto/rand"
	"log"
	"os"

	"github.com/ProtonMail/go-crypto/openpgp"
	"github.com/ProtonMail/go-crypto/openpgp/armor"
	"github.com/ProtonMail/go-crypto/openpgp/packet"
)

const (
	// dir is where the fixture is written, relative to the repository root.
	dir = "internal/deps/testdata/signature"

	// payloadSize is how many bytes are signed. It is larger than any buffer
	// the verifier is expected to use, so the fixture also shows the
	// verification path reading a stream rather than a fixed amount of memory.
	payloadSize = 256 << 10

	// rsaBits is the key size. The publisher signs with RSA 4096; 2048 is used
	// here because the fixture only has to prove the library verifies a
	// detached RSA signature, and a smaller key keeps generation fast enough to
	// run by hand.
	rsaBits = 2048
)

func main() {
	entity, err := openpgp.NewEntity(
		"pisquared fixture",
		"a key generated only to sign this repository's test fixture",
		"fixture@example.invalid",
		&packet.Config{RSABits: rsaBits},
	)
	if err != nil {
		log.Fatal(err)
	}

	// The payload is a deterministic byte stream rather than a few words of
	// text, so that a signature over a short string cannot stand in for one
	// over a file the size of an image.
	payload := make([]byte, payloadSize)
	if _, err := rand.Read(payload); err != nil {
		log.Fatal(err)
	}
	if err := os.WriteFile(dir+"/payload.bin", payload, 0o644); err != nil {
		log.Fatal(err)
	}

	var sig bytes.Buffer
	if err := openpgp.ArmoredDetachSign(&sig, entity, bytes.NewReader(payload), nil); err != nil {
		log.Fatal(err)
	}
	if err := os.WriteFile(dir+"/payload.bin.asc", sig.Bytes(), 0o644); err != nil {
		log.Fatal(err)
	}

	var pub bytes.Buffer
	armorWriter, err := armor.Encode(&pub, openpgp.PublicKeyType, nil)
	if err != nil {
		log.Fatal(err)
	}
	if err := entity.Serialize(armorWriter); err != nil {
		log.Fatal(err)
	}
	if err := armorWriter.Close(); err != nil {
		log.Fatal(err)
	}
	if err := os.WriteFile(dir+"/signer.pub.asc", pub.Bytes(), 0o644); err != nil {
		log.Fatal(err)
	}

	// The signing key's ID is printed so the test's expectation about which key
	// signed the fixture can be checked against the generation, not only
	// against itself.
	for _, subkey := range entity.Subkeys {
		if subkey.Sig.FlagSign {
			log.Printf("signed by subkey %X", subkey.PublicKey.KeyId)
		}
	}
	log.Printf("primary key %X", entity.PrimaryKey.KeyId)
	log.Printf("wrote payload.bin (%d bytes), payload.bin.asc, signer.pub.asc", payloadSize)
}
