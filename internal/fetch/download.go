package fetch

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"hash"
	"io"
	"net/http"
	"os"
	"path/filepath"
)

// bufferSize bounds how much of a 200 MiB transfer is held at once. The
// specification's claim that peak memory is bounded by the buffer rather than
// the image is only true if the buffer is a constant, so it is written as one.
const bufferSize = 32 << 10

// PartSuffix is appended to the output path to name the in-progress file.
//
// It sits beside the output rather than in a temporary directory because a
// rename is atomic within a directory and not across one, and because the file
// a failed transfer left behind is one an operator can find and delete.
const PartSuffix = ".part"

// Progress is how far a transfer has got, reported as it goes so a slow link
// presents as progress rather than as a hang.
type Progress struct {
	// Written is how many bytes are now on disk. It is the size of the part
	// file, because this transfer always writes all of what it receives.
	Written int64

	// Total is how many bytes the publisher says this file is, or 0 when it did
	// not say. A transfer with no total reports progress as bytes alone rather
	// than as a fraction that would be a lie.
	Total int64
}

// Result is a completed transfer: where the bytes are, and what was verified.
type Result struct {
	// Path is where the file now is. It is the output path asked for whenever
	// the transfer completed, whether the bytes were verified or reported as
	// unverified — the two differ in what the caller is told, not in whether a
	// file exists. It is the output path with PartSuffix, and not the output
	// path itself, whenever the transfer did not complete.
	Path string

	// Bytes is the size of the file at Path.
	Bytes int64

	// Checksum is the verification. A caller that proceeds past an unverified
	// download must say so, which is what the State is for.
	Checksum Checksum
}

// Download fetches the file at imageURL to outputPath, verifies it against the
// checksum the publisher publishes for it, and leaves it at outputPath only if
// every byte arrived.
//
// The bytes go to outputPath + PartSuffix and are renamed onto outputPath once
// the whole file is present. A transfer that is interrupted, cancelled, or
// failed therefore leaves nothing at the output path that could be mistaken for
// a finished image, which the specification asks for of all three alike.
//
// Verification happens before the rename, and its outcome decides what happens
// next:
//
//   - Verified: the file is renamed into place.
//   - Unverified: the published checksum could not be read. The file is still
//     placed, because the specification permits an explicitly unverified
//     download, and the returned Result carries the reason so nothing can read
//     it as a pass.
//   - Mismatch: the part file is removed and the returned error names both the
//     published and the observed digest. Removing it is deliberate: keeping
//     bytes that failed their checksum is keeping bytes no one can use.
//
// A part file left by an earlier attempt is discarded rather than continued.
// Resumption was considered and deliberately not built — the image is 200 MiB
// served from a permalink, a retry costs one download, and a resumption is a
// second write path through the same file with the hash of a partial transfer
// to keep correct. Starting over is the branch the specification names as
// acceptable and the one that fails obviously.
func Download(ctx context.Context, client *http.Client, imageURL, outputPath string, onProgress func(Progress)) (Result, error) {
	source := imageURL + Suffix
	partPath := outputPath + PartSuffix

	file, err := openPart(partPath)
	if err != nil {
		return Result{Checksum: unverified(source, err)}, err
	}
	// The safety net for every path that returns before the close below: the
	// transfer error branches need the handle released without saying so each
	// time. Once the explicit close has run, this one only ever closes an
	// already-closed file, which returns an error nothing reads.
	defer file.Close()

	hasher := sha256.New()
	written, total, err := transfer(ctx, client, imageURL, file, hasher, onProgress)
	if err != nil {
		// The part file stays, deliberately: it is nowhere near the output
		// path, and it is what a retry discards.
		return partial(partPath, written, unverified(source, err)), err
	}
	// A transfer that announced a length and did not reach it is reported as
	// the failure it is. A server that closes early is the failure this
	// catches, and it reaches the operator as a named one instead of as an
	// image that boots until it does not.
	if total > 0 && written < total {
		short := fmt.Errorf("%s: the transfer stopped at %d of the %d bytes the publisher declared",
			DescribeSource(source), written, total)
		return partial(partPath, written, unverified(source, short)), short
	}

	if err := file.Sync(); err != nil {
		return partial(partPath, written, unverified(source, err)), err
	}

	// The handle is closed before anything renames or removes the part file
	// below. POSIX systems allow a rename or a removal over an open file,
	// which is why this package's tests pass on Linux and macOS unchanged,
	// but Windows refuses both while a handle is out and answers a sharing
	// violation — the rename of a finished download and the removal of a
	// failed one are the two places that shows. Closing first is correct on
	// every platform, and the hashing needed nothing from the file after the
	// transfer: the digest was computed as the bytes went past.
	if err := file.Close(); err != nil {
		return partial(partPath, written, unverified(source, err)), err
	}

	published, err := Retrieve(ctx, client, imageURL)
	if err != nil {
		return Result{Path: partPath, Bytes: written, Checksum: published}, err
	}
	checked := published.Compare(hex.EncodeToString(hasher.Sum(nil)))

	if checked.State == Mismatch {
		os.Remove(partPath)
		return Result{Path: partPath, Bytes: written, Checksum: checked}, checked
	}

	if err := os.Rename(partPath, outputPath); err != nil {
		return Result{Path: partPath, Bytes: written, Checksum: checked}, err
	}
	return Result{Path: outputPath, Bytes: written, Checksum: checked}, nil
}

// transfer streams the publisher's response into file, hashing and reporting as
// it goes. It returns how many bytes arrived and the total the publisher
// declared, which is 0 when it declared nothing.
func transfer(ctx context.Context, client *http.Client, imageURL string, file *os.File, hasher hash.Hash, onProgress func(Progress)) (written, total int64, err error) {
	if client == nil {
		client = http.DefaultClient
	}

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, imageURL, nil)
	if err != nil {
		return 0, 0, err
	}
	resp, err := client.Do(req)
	if err != nil {
		return 0, 0, err
	}
	// Closing the body is what releases the connection. A downloader that
	// leaked one per attempt would exhaust the pool partway through.
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return 0, 0, &StatusError{URL: imageURL, Status: resp.Status}
	}

	total = declaredTotal(resp)
	written, err = pump(ctx, resp, file, hasher, total, onProgress)
	return written, total, err
}

// pump copies the body into the file, hashing and reporting as it goes. ctx is
// checked between reads rather than only at the start, so an operator who
// cancels a 200 MiB transfer over a slow link stops the transfer instead of
// waiting politely for the rest of it.
func pump(ctx context.Context, resp *http.Response, file *os.File, hasher hash.Hash, total int64, onProgress func(Progress)) (int64, error) {
	var written int64
	if onProgress != nil {
		onProgress(Progress{Written: written, Total: total})
	}

	buf := make([]byte, bufferSize)
	for {
		n, readErr := resp.Body.Read(buf)
		if n > 0 {
			if _, err := file.Write(buf[:n]); err != nil {
				return written, err
			}
			hasher.Write(buf[:n])
			written += int64(n)
			if onProgress != nil {
				onProgress(Progress{Written: written, Total: total})
			}
		}
		if readErr == io.EOF {
			return written, nil
		}
		if readErr != nil {
			return written, readErr
		}
		if err := ctx.Err(); err != nil {
			return written, err
		}
	}
}

// declaredTotal is how many bytes the publisher says the file is, or 0 when it
// said nothing. A streamed response with no length is accepted rather than
// rejected: the verification stage is what decides whether the bytes that
// arrived are the image, and a publisher is not obliged to announce a size.
func declaredTotal(resp *http.Response) int64 {
	if resp.ContentLength < 0 {
		return 0
	}
	return resp.ContentLength
}

// openPart creates the part file, empty, and leaves it positioned at its start.
//
// Any file already there is discarded by the truncation, because a transfer
// that starts over starts over: the bytes of an interrupted one are not a
// prefix of anything this attempt will produce.
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
	if _, err := file.Seek(0, io.SeekStart); err != nil {
		file.Close()
		return nil, err
	}
	return file, nil
}

// unverified is a Checksum for a run that could not be verified at all, with
// the reason kept rather than reduced to a boolean.
func unverified(source string, err error) Checksum {
	c := Checksum{State: Unverified, Source: source}
	if err != nil {
		c.Reason = err.Error()
	}
	return c
}

// partial is a Result for a transfer that did not finish: the file is still at
// its temporary name, which is the whole of what keeps it from being mistaken
// for a finished image.
func partial(path string, bytes int64, checksum Checksum) Result {
	return Result{Path: path, Bytes: bytes, Checksum: checksum}
}

// StatusError is a response the transfer could not use: the publisher answered
// something other than the bytes.
type StatusError struct {
	URL    string
	Status string
}

func (e *StatusError) Error() string {
	return fmt.Sprintf("%s answered %s", e.URL, e.Status)
}
