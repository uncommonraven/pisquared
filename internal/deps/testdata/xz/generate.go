// Command generate writes stream.xz, the fixture that internal/deps proves the
// xz decoder streams with.
//
// It is kept beside the fixture rather than run from a test because the point of
// the fixture is that it exists on disk at a known size. Regenerate it with:
//
//	go run ./internal/deps/testdata/xz/generate.go
//
// The bytes it writes are defined by content() below. That definition is written
// out a second time, independently, in xz_test.go; if the two ever disagree the
// test fails and this file is what needs fixing. They are duplicated on purpose,
// so that the test compares the decoder's output against a statement of what was
// encoded rather than against a value this same program also recorded.
package main

import (
	"log"
	"os"

	"github.com/ulikunitz/xz"
)

const (
	// blockSize is how much of the fixture is generated and compressed at a
	// time, and how far apart two blocks sit in the finished file.
	blockSize = 64 << 10

	// blocks is how many blocks make up the fixture. 1024 blocks is 64 MiB of
	// decompressed data against a 64 KiB decode buffer, which is the ratio that
	// makes the streaming claim testable rather than asserted.
	blocks = 1024

	// dictCap is the dictionary the fixture is compressed with. It is stated
	// here rather than left at the default so the decoder's memory use is a
	// known quantity instead of whatever the library happens to pick.
	dictCap = 1 << 20
)

func main() {
	f, err := os.Create("internal/deps/testdata/xz/stream.xz")
	if err != nil {
		log.Fatal(err)
	}
	defer f.Close()

	w, err := (&xz.WriterConfig{DictCap: dictCap}).NewWriter(f)
	if err != nil {
		log.Fatal(err)
	}

	buf := make([]byte, blockSize)
	// One block of splitmix64 bytes, written out repeatedly. Repeating it is
	// what makes the fixture compress to a small file while still decompressing
	// to 64 MiB, which is the ratio the streaming proof needs.
	var state uint64
	fill(buf, &state)

	block := append([]byte(nil), buf...)
	for i := range blocks {
		copy(buf, block)
		// Each block's first byte carries its index, so no two blocks are
		// identical and the fixture is not one block written a thousand times.
		buf[0] = byte(i)
		if _, err := w.Write(buf); err != nil {
			log.Fatal(err)
		}
	}

	if err := w.Close(); err != nil {
		log.Fatal(err)
	}
	log.Printf("wrote %d blocks of %d bytes", blocks, blockSize)
}

// content fills buf with splitmix64 bytes from a fixed seed, so the fixture is
// reproducible and the test can produce the same bytes without reading it.
func fill(buf []byte, state *uint64) {
	*state = 1
	for i := range buf {
		*state += 0x9e3779b97f4a7c15
		z := *state
		z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9
		z = (z ^ (z >> 27)) * 0x94d049bb133111eb
		buf[i] = byte(z ^ (z >> 31))
	}
}
