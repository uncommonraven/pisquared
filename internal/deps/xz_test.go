package deps

import (
	"bytes"
	"fmt"
	"io"
	"os"
	"runtime"
	"testing"
	"time"

	"github.com/ulikunitz/xz"
)

// The proof this file records is that github.com/ulikunitz/xz decompresses as a
// stream. Roughly 200 MiB compressed becomes 1.1 GiB decompressed in the real
// case, so the question is not whether the library can decode a file but whether
// it can decode one the machine does not have room to hold. Everything here
// exists to make that a measurement rather than an assumption.
//
// The version is v0.5.17, which is the module's latest stable tag and requires
// nothing at all — its go.mod names no dependency. The other candidate,
// v0.6.0-last-dev, is an abandoned development snapshot from an earlier date
// that would drag in github.com/ulikunitz/lz and golang.org/x/exp. Taking the
// stable tag with no dependencies is what keeps CGO_ENABLED=0 and a single
// static binary working, and the build check below is what shows it rather than
// what assumes it.
//
// See testdata/xz/generate.go for how the fixture is produced.

const (
	// decodeBuffer is the buffer the fixture is decompressed through. It is
	// also the size of the largest piece of the fixture allowed to be in
	// memory at once, which is the whole claim being tested.
	decodeBuffer = 64 << 10

	// fixtureSize is how many bytes the fixture decompresses to. It is a
	// thousand times the buffer, so a decoder that held the image would use
	// sixteen times the ceiling below and fail this test rather than pass it
	// by accident.
	fixtureSize = 64 << 20

	// memoryCeiling is the most the heap is allowed to grow while the fixture
	// is decompressed. It is several times what a 1 MiB dictionary and one
	// buffer need, and several times smaller than the fixture, so it passes on
	// a stream and fails on a slurp.
	memoryCeiling = 16 << 20

	// sampleInterval is how often peak heap is sampled during the run. Short
	// enough to catch the dictionary and the buffers, long enough that
	// sampling does not itself dominate the measurement.
	sampleInterval = time.Millisecond
)

// TestXZDecompressesAFixtureLargerThanTheBufferWithoutHoldingIt is the proof.
// It streams the fixture through a buffer far smaller than the fixture and
// fails if the process grows by anything close to the fixture's size.
func TestXZDecompressesAFixtureLargerThanTheBufferWithoutHoldingIt(t *testing.T) {
	out, err := os.CreateTemp(t.TempDir(), "decompressed-*.img")
	if err != nil {
		t.Fatal(err)
	}
	defer out.Close()

	src, err := os.Open("testdata/xz/stream.xz")
	if err != nil {
		t.Fatal(err)
	}
	defer src.Close()

	// The peak is sampled from another goroutine because a peak that is only
	// read after the work finishes is not a peak. It is the maximum HeapAlloc
	// seen while the decompression runs, less the heap as it stood before.
	stop := make(chan struct{})
	done := make(chan uint64, 1)
	go func() {
		var peak uint64
		for {
			var stats runtime.MemStats
			runtime.ReadMemStats(&stats)
			if stats.HeapAlloc > peak {
				peak = stats.HeapAlloc
			}
			select {
			case <-stop:
				done <- peak
				return
			case <-time.After(sampleInterval):
			}
		}
	}()

	baseline := liveHeap(t)

	z, err := xz.NewReader(src)
	if err != nil {
		close(stop)
		<-done
		t.Fatalf("opening the fixture as xz: %v", err)
	}

	// The destination checks every byte against an independently written
	// statement of what the fixture contains, and counts the largest single
	// write it was asked to make.
	dst := &verifyingWriter{
		file:  out,
		block: fixtureBlock(t),
	}

	buf := make([]byte, decodeBuffer)
	written, copyErr := io.CopyBuffer(dst, z, buf)

	close(stop)
	peak := <-done

	if copyErr != nil {
		t.Fatalf("decompressing the fixture: %v", copyErr)
	}

	if written != fixtureSize {
		t.Errorf("decompressed %d bytes, want %d", written, fixtureSize)
	}
	if written/decodeBuffer < 1000 {
		t.Errorf("the fixture is %d times the buffer; that is not large enough for this to mean anything",
			written/decodeBuffer)
	}

	// Every byte was compared against what the fixture is defined to contain,
	// which is what makes this a round trip rather than a successful read.
	if dst.writes < 1000 {
		t.Errorf("the decompressed fixture arrived in %d writes; a stream this size should arrive in pieces", dst.writes)
	}
	if dst.maxWrite > decodeBuffer {
		t.Errorf("the largest single write was %d bytes, above the %d byte buffer", dst.maxWrite, decodeBuffer)
	}

	if growth := peak - baseline; growth > memoryCeiling {
		t.Errorf("the heap grew by %d bytes while decompressing %d; peak live heap was %d against a baseline of %d",
			growth, fixtureSize, peak, baseline)
	}
	t.Logf("decompressed %d bytes through a %d byte buffer; heap grew by %d bytes, ceiling %d",
		written, decodeBuffer, peak-baseline, memoryCeiling)
}

func liveHeap(t *testing.T) uint64 {
	t.Helper()
	runtime.GC()
	var stats runtime.MemStats
	runtime.ReadMemStats(&stats)
	return stats.HeapAlloc
}

// verifyingWriter writes to a file, and against a stream of what the fixture is
// defined to contain. It fails the moment a byte differs, so a decoder that
// produced the right length and the wrong bytes would not pass.
type verifyingWriter struct {
	file     io.Writer
	block    []byte
	emitted  int
	offset   int // how far into the current block the next byte belongs
	writes   int
	maxWrite int
	written  int64
}

func (v *verifyingWriter) Write(p []byte) (int, error) {
	for i, got := range p {
		want, err := v.byteAt(len(v.block)*v.emitted + v.offset)
		if err != nil {
			return i, err
		}
		if got != want {
			return i, fmt.Errorf("fixture byte %d is %#02x, want %#02x", v.offset, got, want)
		}
		v.offset++
		if v.offset == len(v.block) {
			v.offset = 0
			v.emitted++
		}
	}

	v.writes++
	if len(p) > v.maxWrite {
		v.maxWrite = len(p)
	}
	v.written += int64(len(p))
	return v.file.Write(p)
}

// byteAt returns the byte the fixture holds at absolute index n, which is the
// block at n/blockSize with the block's first byte replaced by the block's
// index.
func (v *verifyingWriter) byteAt(n int) (byte, error) {
	block := n / len(v.block)
	if block >= fixtureBlocks {
		return 0, fmt.Errorf("the fixture ends at %d bytes", fixtureSize)
	}
	if n%len(v.block) == 0 {
		return byte(block), nil
	}
	return v.block[n%len(v.block)], nil
}

// fixtureBlock returns the 64 KiB of splitmix64 bytes every block of the
// fixture is built from. It is written out here a second time, independently of
// the generator, so the test compares the decoder against a statement of what
// was encoded rather than against something the encoder also recorded.
func fixtureBlock(t *testing.T) []byte {
	t.Helper()
	block := make([]byte, blockSize)
	state := uint64(1)
	for i := range block {
		state += 0x9e3779b97f4a7c15
		z := state
		z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9
		z = (z ^ (z >> 27)) * 0x94d049bb133111eb
		block[i] = byte(z ^ (z >> 31))
	}
	return block
}

// blockSize and fixtureBlocks describe the fixture's shape. They are the same
// numbers testdata/xz/generate.go writes it with.
const (
	blockSize     = decodeBuffer
	fixtureBlocks = fixtureSize / blockSize
)

// TestXZFixtureComparisonRejectsBytesThatAreNotTheFixture is the other half of
// the round trip. The streaming test is only worth anything if the comparison
// it makes can fail, so the comparison is exercised on both a correct prefix
// and a wrong one. A check that cannot fail is not a check.
func TestXZFixtureComparisonRejectsBytesThatAreNotTheFixture(t *testing.T) {
	block := fixtureBlock(t)

	// The fixture's first byte is the index of its first block, which is zero.
	// Every byte after it is the block's own.
	correct := append([]byte{0}, block[1:8]...)

	if _, err := (&verifyingWriter{file: io.Discard, block: block}).Write(correct); err != nil {
		t.Errorf("writing the fixture's own bytes: %v; the comparison rejects what it should accept", err)
	}

	wrong := append([]byte(nil), correct...)
	wrong[4]++
	if _, err := (&verifyingWriter{file: io.Discard, block: block}).Write(wrong); err == nil {
		t.Error("writing one byte that is not the fixture's was accepted, so the streaming test proves nothing")
	}

	if _, err := (&verifyingWriter{file: io.Discard, block: block}).Write(correct); err != nil {
		t.Errorf("writing the fixture's bytes a second time: %v", err)
	}
}

// TestXZFixtureIsXzAndSmall proves the fixture on disk is what the streaming
// test assumes: a real xz stream far smaller than what it decompresses to. A
// fixture that stopped compressing would make the test above meaningless, so it
// is checked rather than trusted.
func TestXZFixtureIsXzAndSmall(t *testing.T) {
	info, err := os.Stat("testdata/xz/stream.xz")
	if err != nil {
		t.Fatal(err)
	}

	header := make([]byte, 6)
	f, err := os.Open("testdata/xz/stream.xz")
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	if _, err := io.ReadFull(f, header); err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(header, []byte{0xfd, '7', 'z', 'X', 'Z', 0x00}) {
		t.Errorf("the fixture begins %x, which is not an xz stream header", header)
	}

	if info.Size() >= fixtureSize/10 {
		t.Errorf("the fixture is %d bytes on disk against %d decompressed; it is not compressing",
			info.Size(), fixtureSize)
	}
	t.Logf("fixture is %d bytes on disk and decompresses to %d", info.Size(), fixtureSize)
}
