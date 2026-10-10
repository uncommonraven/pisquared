package decompress

import (
	"context"
	"fmt"
	"io"
	"os"
	"path/filepath"

	"github.com/ulikunitz/xz"
)

// bufferSize bounds how much of a 1.1 GiB image is held at once. The image is
// never resident only if this is a constant, so it is written as one, and it is
// the size the transfer in fetch uses so that the two stages of one run behave
// alike.
const bufferSize = 32 << 10

// PartSuffix is appended to the output path to name the in-progress file.
//
// It sits beside the output rather than in a temporary directory because a
// rename is atomic within a directory and not across one. fetch keeps its own
// copy of this constant rather than the two packages sharing one: they are
// separate stages of a run, neither imports the other, and the convention each
// follows is the same either way.
const PartSuffix = ".part"

// Limitation is what the tool's output says beside a decompressed image, and is
// the sentence that keeps this step honest.
//
// The publisher publishes no checksum and no size for any decompressed image, so
// nothing here compares the result against a published value: there is none to
// compare against, and a verification stage with nothing to check is a check
// that always passes. What is established instead is that the whole stream was
// consumed, that the image was never held in memory, and that the file at the
// output path was renamed into place only once every byte had been written. That
// is a statement about the operation rather than about the bytes, and section 7
// carries this wording in whatever terms its step log renders it in.
const Limitation = "decompression was not verified against a published value: the publisher publishes none for the " +
	"decompressed image, so this was proved by construction instead — the whole stream was consumed in a single " +
	"pass, the image was never held in memory, and it appeared at the output path only once it was complete"

// Result is a completed decompression.
type Result struct {
	// Source is the compressed file that was expanded.
	Source string

	// Path is where the flashable image now is, and is always the output path
	// asked for: a decompression that did not complete returns no Result at all
	// and leaves nothing there.
	Path string

	// Bytes is how large the decompressed image is. It is the count the decoder
	// produced rather than a size anybody published, because nobody publishes
	// one, and it is reported as a count for that reason.
	Bytes int64
}

// Error is a stream that did not decompress, and is returned as the error that
// halts a run the way a checksum mismatch and a bad signature are: a caller that
// checks for a non-nil error then stops, so nothing partial is presented as a
// flashable image.
type Error struct {
	// Source is the compressed file that failed.
	Source string

	// Reason is what the decoder said, kept verbatim. A stream that ended early
	// and a block that failed its own check are different faults, and the words
	// the decoder uses say which.
	Reason string

	// Written is how many bytes reached the temporary file before the failure.
	// They are gone by the time this error is returned; the count is kept so a
	// report can say how much of an image was discarded.
	Written int64
}

func (e *Error) Error() string {
	return fmt.Sprintf("%s did not decompress: %s", filepath.Base(e.Source), e.Reason)
}

// Stream expands the xz stream at compressedPath and writes the image to
// outputPath, leaving it there only if the whole stream decompressed.
//
// The bytes go to outputPath + PartSuffix and are renamed onto outputPath once
// the decoder has reached the end of the stream, which is the only place the xz
// format says an image ended. A stream that is truncated, corrupt, or not xz at
// all therefore leaves nothing at the output path that could be mistaken for a
// finished image.
//
// It leaves nothing at the part path either, which is the one place this
// deliberately differs from fetch: a partial decompression records nothing
// anybody can act on, is five times the size of the file it came from, and
// cannot be resumed the way a transfer can. A run that fails here starts the
// decompression over, exactly as a run whose download was cut off fetches the
// image again.
//
// The image is read through a constant-sized buffer and written as it is decoded,
// so nothing here holds the image: what is resident is that buffer plus whatever
// the decoder needs for the stream, which measured against the publisher's own
// RPi5 image is roughly 120 MiB of live heap against 1.1 GiB decompressed. That
// cost is the decoder's and not this package's, and doc.go records it as a
// measured number rather than as a bound this code can promise on its own.
//
// Nothing here is verified against a published value, because none is published;
// see Limitation for what is said instead.
//
// ctx stops a decompression that has already started, which is what makes a
// cancellation in the interface stop the whole acquisition rather than only the
// transfer before it.
func Stream(ctx context.Context, compressedPath, outputPath string) (Result, error) {
	partPath := outputPath + PartSuffix

	file, err := openPart(partPath)
	if err != nil {
		return Result{}, err
	}

	written, err := expand(ctx, compressedPath, file)
	// The bytes reach the disk before the rename and not after, so a failure
	// between the two leaves nothing at the output path that a rename never
	// reached.
	if syncErr := file.Sync(); err == nil {
		err = syncErr
	}
	if closeErr := file.Close(); err == nil {
		err = closeErr
	}

	// One cleanup path for every failure, cancellation included. The first error
	// is the one reported and the ones after it are the same failure seen from
	// further along.
	if err != nil {
		os.Remove(partPath)
		return Result{}, err
	}
	if err := os.Rename(partPath, outputPath); err != nil {
		os.Remove(partPath)
		return Result{}, err
	}
	return Result{Source: compressedPath, Path: outputPath, Bytes: written}, nil
}

// expand streams the compressed file at source through the decoder into dest,
// and returns how many bytes it produced.
//
// The loop is written out rather than handed to io.CopyBuffer because ctx is
// checked between reads rather than only at the start: an operator who cancels a
// decompression stops it instead of waiting for the rest of the image. It is the
// same shape as fetch's pump, for the same reason.
//
// An error from the decoder is a stream that did not decompress and is returned
// as one. An error from opening the source is a file that could not be read and
// is returned as it happened, because a missing file is a missing file and not a
// corrupt stream.
func expand(ctx context.Context, source string, dest io.Writer) (int64, error) {
	compressed, err := os.Open(source)
	if err != nil {
		return 0, err
	}
	defer compressed.Close()

	// The format is identified by the decoder rather than by the filename. A
	// file that is not xz fails here, which is the honest answer for one: the
	// publisher serves .xz, and anything else beside that name is a file that
	// did not arrive.
	decoder, err := xz.NewReader(compressed)
	if err != nil {
		return 0, &Error{Source: source, Reason: err.Error()}
	}

	var written int64
	buf := make([]byte, bufferSize)
	for {
		if err := ctx.Err(); err != nil {
			return written, err
		}
		n, readErr := decoder.Read(buf)
		if n > 0 {
			if _, err := dest.Write(buf[:n]); err != nil {
				return written, err
			}
			written += int64(n)
		}
		if readErr == io.EOF {
			return written, nil
		}
		if readErr != nil {
			return written, &Error{Source: source, Reason: readErr.Error(), Written: written}
		}
	}
}

// openPart creates the part file, empty, ready to be written from its start.
//
// Any file already there is discarded by the truncation, because a decompression
// that starts over starts over: the tail of an earlier attempt is not a prefix of
// this one, and appending to it would produce an image that is neither.
func openPart(partPath string) (*os.File, error) {
	if dir := filepath.Dir(partPath); dir != "" && dir != "." {
		if err := os.MkdirAll(dir, 0o755); err != nil {
			return nil, fmt.Errorf("cannot create %s: %w", dir, err)
		}
	}
	// Opened for reading and writing rather than appending, because the whole
	// file is written from offset zero.
	file, err := os.OpenFile(partPath, os.O_CREATE|os.O_RDWR|os.O_TRUNC, 0o644)
	if err != nil {
		return nil, fmt.Errorf("cannot open %s: %w", partPath, err)
	}
	return file, nil
}
