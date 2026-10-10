// The tests in this file cover section 6 against a fixture this file generates
// rather than against the publisher's image, so that go test ./... neither spends
// 200 MiB of somebody's bandwidth nor fails because dietpi.com is slow.
//
// The fixture is generated rather than committed because the property under test
// is that the decompressor streams, and a fixture on disk is a file somebody has
// to regenerate by hand whenever its definition changes. What it compresses to is
// far smaller than what it decompresses to, which is the ratio the memory claim
// is made over.
//
// Two fixtures are used. The streaming one is 64 MiB, a thousand times the buffer
// the decompressor reads it through, because a bounded-memory claim needs the
// fixture to be large enough for a slurp to be visible in the heap. The failure
// one is 2 MiB, because truncation and corruption do not depend on size and the
// cases that exist to fail should stay fast.
package decompress_test

import (
	"bytes"
	"context"
	"crypto/sha256"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"

	"github.com/ulikunitz/xz"
	"github.com/uncommonraven/pisquared/internal/decompress"
)

const (
	// blockSize is how much of the fixture is composed at a time, and how far
	// apart two blocks sit in the finished stream. It is also the unit the
	// fixture's content is defined in.
	blockSize = 64 << 10

	// fixtureDictCap is the dictionary the fixture is compressed with. Stated
	// rather than left at the default so the decoder's memory use is a known
	// quantity instead of whatever the library happens to pick.
	fixtureDictCap = 1 << 20

	// streamingBlocks is how many blocks make up the fixture the memory claim is
	// made over: 1024 blocks is 64 MiB decompressed.
	streamingBlocks = 1024

	// failureBlocks is how many blocks make up the fixture the failure cases run
	// over: 32 blocks is 2 MiB.
	failureBlocks = 32

	// memoryCeiling is the most the heap is allowed to grow while the streaming
	// fixture is decompressed. It is several times what a 1 MiB dictionary and
	// one buffer need, and four times smaller than the fixture, so it passes on
	// a stream and fails on a slurp.
	memoryCeiling = 16 << 20

	// decodeBuffer is the size the decompressor reads through. It is written out
	// here rather than imported because the package keeps it unexported, and it
	// is what the fixture's size is chosen against.
	decodeBuffer = 32 << 10

	// sampleInterval is how often the heap is sampled while work runs. Short
	// enough to catch the dictionary and the buffers, long enough that sampling
	// does not itself dominate the measurement.
	sampleInterval = time.Millisecond

	// streamingDigest and failureDigest are the sha256 of the plaintext each
	// fixture is defined to decompress to.
	//
	// They are pinned rather than computed from the generator, because a test
	// that computes its own expectation proves only that the decompressor
	// inverted whatever the compressor was handed. With them pinned, a
	// decompressor that dropped, repeated, or reordered a block fails here even
	// if the generator that built the fixture is itself wrong.
	streamingDigest = "4ca4692b5c1b9732f1a5654937a98a9e165ebf3edcdde694f9cd1db58fecb6a3"
	failureDigest   = "78db5da2d023db242ae1c650039f041a4b69f6931301f9bd590dedfaaec53d54"

	// imageName is what the publisher calls an image, which is what the output
	// path beside it is named after.
	imageName = "DietPi_RPi5-ARMv8-Trixie.img"
)

// TestStreamingAFixtureLargerThanTheMemoryCeilingCompletes is task 6.1. The
// fixture is a thousand times the buffer the decompressor reads it through, and
// the run completes without the heap growing by anything close to the fixture's
// size — which is the bounded-memory claim, measured rather than asserted.
func TestStreamingAFixtureLargerThanTheMemoryCeilingCompletes(t *testing.T) {
	dir := t.TempDir()
	src := filepath.Join(dir, imageName+".xz")
	out := filepath.Join(dir, imageName)

	digest, size := writeFixture(t, src, streamingBlocks)
	if size != streamingBlocks*blockSize {
		t.Fatalf("the fixture is %d bytes, want the %d it is defined as", size, streamingBlocks*blockSize)
	}
	if digest != streamingDigest {
		t.Fatalf("the fixture's plaintext hashes to %s, want the %s it is defined to decompress to;\n"+
			"if the fixture's definition changed, these constants are what needs updating",
			digest, streamingDigest)
	}
	if size/decodeBuffer < 1000 {
		t.Fatalf("the fixture is %d times the buffer; that is not large enough for the ceiling below to mean anything",
			size/decodeBuffer)
	}

	// The claim rests on the compressed form being far smaller than what it
	// decompresses to, so it is checked rather than trusted: a fixture that
	// stopped compressing would make the ceiling below meaningless.
	if info, err := os.Stat(src); err != nil {
		t.Fatal(err)
	} else if info.Size() >= size/10 {
		t.Errorf("the fixture is %d bytes on disk against %d decompressed; it is not compressing",
			info.Size(), size)
	}

	var result decompress.Result
	peak, baseline := peakHeap(t, func() {
		var err error
		result, err = decompress.Stream(context.Background(), src, out)
		if err != nil {
			t.Errorf("decompressing the fixture: %v", err)
		}
	})

	if result.Path != out {
		t.Errorf("the image is at %q, want %q", result.Path, out)
	}
	if result.Source != src {
		t.Errorf("the result names %q as its source, want %q", result.Source, src)
	}
	if result.Bytes != size {
		t.Errorf("the result reports %d bytes, want %d", result.Bytes, size)
	}

	got, gotSize := digestOfFile(t, out)
	if got != digest {
		t.Errorf("the image at the output path hashes to %s, want the fixture's %s;\n"+
			"a decompression that dropped, repeated, or reordered a block lands here", got, digest)
	}
	if gotSize != size {
		t.Errorf("the image at the output path is %d bytes, want %d", gotSize, size)
	}
	// The part file is gone once the stream has ended, or the output directory
	// carries two copies of a 1.1 GiB file forever.
	if _, err := os.Stat(out + decompress.PartSuffix); !os.IsNotExist(err) {
		t.Errorf("%q is still there after a completed decompression", out+decompress.PartSuffix)
	}

	if baseline > peak {
		// The heap shrank while the work ran, which makes the growth below zero
		// rather than a measurement of anything.
		peak = baseline
	}
	if growth := peak - baseline; growth > memoryCeiling {
		t.Errorf("the heap grew by %d bytes while decompressing %d; peak live heap was %d against a baseline of %d",
			growth, size, peak, baseline)
	} else {
		t.Logf("decompressed %d bytes; the heap grew by %d bytes against a ceiling of %d",
			size, growth, memoryCeiling)
	}
}

// TestAStreamThatDoesNotDecompressLeavesNothingAtTheOutputPath is task 6.2. Every
// case below is a stream that stops short of where the format says an image
// ended, and every one of them must stop the run with the reason shown and leave
// nothing that could be mistaken for a finished image.
func TestAStreamThatDoesNotDecompressLeavesNothingAtTheOutputPath(t *testing.T) {
	whole, _, size := buildFixture(t, failureBlocks)

	cases := []struct {
		name   string
		damage func([]byte) []byte

		// wholeImage marks the cases where every byte of the image decodes and
		// only the stream's end is missing. They are the ones that matter most
		// here: a decompressor that judged success by how much arrived would
		// rename a complete-looking 1.1 GiB file into place.
		wholeImage bool
	}{
		{
			name:   "cut in the middle",
			damage: func(b []byte) []byte { return b[:len(b)/2] },
		},
		{
			name:       "cut before the footer",
			damage:     func(b []byte) []byte { return b[:len(b)-12] },
			wholeImage: true,
		},
		{
			name:       "garbage appended after the stream",
			damage:     func(b []byte) []byte { return append(copyOf(b), 0x00, 0x01, 0x02) },
			wholeImage: true,
		},
		{
			// A byte changed in the middle, which the block's own check catches
			// after the whole image has decoded.
			name:   "corrupt in the middle",
			damage: func(b []byte) []byte { return flip(copyOf(b), len(b)/2) },
		},
		{
			// A byte changed near the end, which the index's checksum catches.
			name:   "corrupt near the foot",
			damage: func(b []byte) []byte { return flip(copyOf(b), len(b)-24) },
		},
		{
			// Not xz at all. The publisher serves .xz, so anything else beside
			// that name is a file that did not arrive.
			name:   "not xz at all",
			damage: func([]byte) []byte { return []byte("this is not an xz stream") },
		},
		{
			name:   "empty",
			damage: func([]byte) []byte { return nil },
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			dir := t.TempDir()
			src := filepath.Join(dir, imageName+".xz")
			out := filepath.Join(dir, imageName)
			if err := os.WriteFile(src, tc.damage(whole), 0o644); err != nil {
				t.Fatalf("writing the damaged fixture: %v", err)
			}

			result, err := decompress.Stream(context.Background(), src, out)
			if err == nil {
				t.Fatal("a damaged stream reported success")
			}

			// It is reported as a decompression error rather than as an
			// unclassified one, so a run halts on it the way it halts on a
			// checksum mismatch.
			var streamErr *decompress.Error
			if !errors.As(err, &streamErr) {
				t.Fatalf("the failure is %T, want a decompression error naming the stream", err)
			}
			if streamErr.Source != src {
				t.Errorf("the error names %q as the stream that failed, want %q", streamErr.Source, src)
			}
			if streamErr.Reason == "" {
				t.Error("the decompression error carries no reason, so a run stops saying nothing")
			}
			if !strings.Contains(err.Error(), "did not decompress") {
				t.Errorf("the report does not say the stream did not decompress:\n%s", err)
			}

			// Nothing at the output path, which is the specification's own
			// scenario, and nothing at the part path either: a partial image is
			// not something to rebuild from.
			if _, statErr := os.Stat(out); !os.IsNotExist(statErr) {
				t.Errorf("a failed decompression left a file at the output path, " +
					"which is indistinguishable from a finished image")
			}
			if _, statErr := os.Stat(out + decompress.PartSuffix); !os.IsNotExist(statErr) {
				t.Errorf("a failed decompression left %q behind, five times the size of the file it came from",
					out+decompress.PartSuffix)
			}
			if result.Path != "" {
				t.Errorf("a failed decompression returned a result naming %q", result.Path)
			}

			if tc.wholeImage && streamErr.Written != size {
				t.Errorf("the stream decoded %d of the fixture's %d bytes; this case is meant to be the one "+
					"where the whole image decodes and only the stream's end is missing",
					streamErr.Written, size)
			}
		})
	}
}

// TestACompressedFileThatIsNotThereIsAnErrorNotAPass covers the call a run makes
// on a path that turns out to hold nothing. Nothing appears at the output path,
// and the failure names the file it could not read.
func TestACompressedFileThatIsNotThereIsAnErrorNotAPass(t *testing.T) {
	dir := t.TempDir()
	src := filepath.Join(dir, imageName+".xz")
	out := filepath.Join(dir, imageName)

	result, err := decompress.Stream(context.Background(), src, out)
	if err == nil {
		t.Fatal("a compressed file that is not there was reported as a successful decompression")
	}
	if !strings.Contains(err.Error(), src) {
		t.Errorf("the failure does not name the file it could not read:\n%s", err)
	}
	if _, statErr := os.Stat(out); !os.IsNotExist(statErr) {
		t.Error("a failed decompression left a file at the output path")
	}
	if _, statErr := os.Stat(out + decompress.PartSuffix); !os.IsNotExist(statErr) {
		t.Errorf("a failed decompression left %q behind", out+decompress.PartSuffix)
	}
	if result.Path != "" {
		t.Errorf("a failed decompression returned a result naming %q", result.Path)
	}
}

// TestACancelledDecompressionLeavesNothingAtTheOutputPath covers the cancellation
// branch's cleanup. It does not measure how promptly a stop arrives mid-image —
// nothing in the package exposes a point to cancel from — but it does show that
// a decompression that was asked to stop leaves nothing at the output path.
func TestACancelledDecompressionLeavesNothingAtTheOutputPath(t *testing.T) {
	dir := t.TempDir()
	src := filepath.Join(dir, imageName+".xz")
	out := filepath.Join(dir, imageName)

	// The fixture is written so that there is a compressed file for the
	// cancellation to land against; its content is not what this test is about.
	writeFixture(t, src, failureBlocks)

	ctx, cancel := context.WithCancel(context.Background())
	cancel()

	if _, err := decompress.Stream(ctx, src, out); !errors.Is(err, context.Canceled) {
		t.Fatalf("a cancelled decompression returned %v, want the cancellation to be reported as one", err)
	}
	if _, statErr := os.Stat(out); !os.IsNotExist(statErr) {
		t.Error("a cancelled decompression left a file at the output path")
	}
	if _, statErr := os.Stat(out + decompress.PartSuffix); !os.IsNotExist(statErr) {
		t.Errorf("a cancelled decompression left %q behind", out+decompress.PartSuffix)
	}
}

// TestThePartFileOfAnEarlierAttemptIsDiscarded covers what a run does over the
// remains of one that was cut off. Appending to it would produce an image that is
// neither the earlier attempt nor this one.
func TestThePartFileOfAnEarlierAttemptIsDiscarded(t *testing.T) {
	dir := t.TempDir()
	src := filepath.Join(dir, imageName+".xz")
	out := filepath.Join(dir, imageName)
	digest, size := writeFixture(t, src, failureBlocks)

	stale := append([]byte("an attempt that was cut off"), make([]byte, 4096)...)
	if err := os.WriteFile(out+decompress.PartSuffix, stale, 0o644); err != nil {
		t.Fatalf("staging the leftover part file: %v", err)
	}

	result, err := decompress.Stream(context.Background(), src, out)
	if err != nil {
		t.Fatalf("decompressing over a leftover part file: %v", err)
	}
	if result.Bytes != size {
		t.Errorf("the result reports %d bytes, want %d", result.Bytes, size)
	}
	if got, _ := digestOfFile(t, out); got != digest {
		t.Errorf("the image at the output path hashes to %s, want the fixture's %s;\n"+
			"a leftover part file was continued rather than discarded", got, digest)
	}
	if _, statErr := os.Stat(out + decompress.PartSuffix); !os.IsNotExist(statErr) {
		t.Errorf("%q is still there after a completed decompression", out+decompress.PartSuffix)
	}
}

// TestTheLimitationSaysWhatWasNotEstablished covers the specification's
// "Decompression is reported honestly" scenario. The scenario is a claim about
// what the tool says, and this constant is the sentence section 7's step log
// carries for it, so a check on the wording is the only way to show it still says
// what it must.
func TestTheLimitationSaysWhatWasNotEstablished(t *testing.T) {
	said := decompress.Limitation

	if !strings.Contains(said, "not verified against a published value") {
		t.Errorf("the limitation does not say the result was not verified against a published value:\n%s", said)
	}
	if !strings.Contains(said, "publishes none") {
		t.Errorf("the limitation does not say why there is nothing to verify against:\n%s", said)
	}
	// What replaces the check is a statement about the operation, and the
	// wording has to keep that distinct from a check that was made.
	for _, want := range []string{"proved by construction", "whole stream", "never held in memory"} {
		if !strings.Contains(said, want) {
			t.Errorf("the limitation does not say %q about what was established instead:\n%s", want, said)
		}
	}
}

// buildFixture returns the bytes of an xz stream of blocks of the fixture's
// content, with the digest and size of the plaintext inside it.
func buildFixture(t *testing.T, blocks int) (compressed []byte, digest string, size int64) {
	t.Helper()

	var buf bytes.Buffer
	w, err := (&xz.WriterConfig{DictCap: fixtureDictCap}).NewWriter(&buf)
	if err != nil {
		t.Fatalf("opening the fixture's compressor: %v", err)
	}

	hasher := sha256.New()
	block := make([]byte, blockSize)
	for i := 0; i < blocks; i++ {
		fixtureBlock(block, i)
		if _, err := w.Write(block); err != nil {
			t.Fatalf("writing block %d: %v", i, err)
		}
		hasher.Write(block)
		size += int64(len(block))
	}
	if err := w.Close(); err != nil {
		t.Fatalf("closing the fixture's compressor: %v", err)
	}
	return buf.Bytes(), fmt.Sprintf("%x", hasher.Sum(nil)), size
}

// writeFixture writes a fixture to path and returns the digest and size of the
// plaintext inside it.
func writeFixture(t *testing.T, path string, blocks int) (digest string, size int64) {
	t.Helper()

	compressed, digest, size := buildFixture(t, blocks)
	if err := os.WriteFile(path, compressed, 0o644); err != nil {
		t.Fatalf("writing the fixture to %s: %v", path, err)
	}
	return digest, size
}

// fixtureBlock fills buf with one block of the fixture's content: splitmix64
// bytes from a fixed seed, with the first byte replaced by the block's index so
// that no two blocks are identical and a decompressor that dropped or repeated
// one would change the digest.
func fixtureBlock(buf []byte, index int) {
	var state uint64 = 1
	for i := range buf {
		state += 0x9e3779b97f4a7c15
		z := state
		z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9
		z = (z ^ (z >> 27)) * 0x94d049bb133111eb
		buf[i] = byte(z ^ (z >> 31))
	}
	buf[0] = byte(index)
}

// digestOfFile streams a file through sha256 and returns its digest and size. It
// reads the file rather than loading it, because the file here is the one the
// decompressor is claimed not to have held.
func digestOfFile(t *testing.T, path string) (string, int64) {
	t.Helper()

	f, err := os.Open(path)
	if err != nil {
		t.Fatalf("opening %s: %v", path, err)
	}
	defer f.Close()

	hasher := sha256.New()
	n, err := io.Copy(hasher, f)
	if err != nil {
		t.Fatalf("reading %s: %v", path, err)
	}
	return fmt.Sprintf("%x", hasher.Sum(nil)), n
}

// peakHeap runs fn while sampling the heap, and returns the largest live heap
// seen alongside it and the heap as it stood before it started.
//
// A peak read after the work finishes is not a peak, which is why the sampling
// runs beside the work rather than after it. The heap is settled first for the
// opposite reason: the fixture's generation leaves buffers behind that nothing
// has collected yet, and a window opened before a collection would measure those
// instead of the work being measured.
func peakHeap(t *testing.T, fn func()) (peak, baseline uint64) {
	t.Helper()

	runtime.GC()
	runtime.GC()

	var before runtime.MemStats
	runtime.ReadMemStats(&before)
	baseline = before.HeapAlloc

	stop := make(chan struct{})
	done := make(chan uint64, 1)
	go func() {
		var highest uint64
		for {
			var stats runtime.MemStats
			runtime.ReadMemStats(&stats)
			if stats.HeapAlloc > highest {
				highest = stats.HeapAlloc
			}
			select {
			case <-stop:
				done <- highest
				return
			case <-time.After(sampleInterval):
			}
		}
	}()

	fn()

	close(stop)
	return <-done, baseline
}

func copyOf(b []byte) []byte { return append([]byte(nil), b...) }

func flip(b []byte, at int) []byte {
	b[at] ^= 0xff
	return b
}
