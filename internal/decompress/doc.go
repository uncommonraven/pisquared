// Package decompress expands the publisher's compressed image into the
// flashable file an operator writes to a card.
//
// This is section 6 of openspec/changes/v0-1-image-download. The publisher
// publishes no checksum and no size for the decompressed file, for any board or
// any release, so there is no published value to check a result against and this
// package does not pretend there is. What it establishes instead is the shape of
// the operation, which is what design.md's "Verification is of the download;
// decompression is proved by construction" puts in place of the check that is not
// available:
//
//   - The whole stream is consumed or nothing is renamed into place. An xz
//     stream truncated near its end still decodes to a file of the expected
//     size, which is why the decoder reaching the end of the stream is what
//     decides success and not the size of what arrived.
//   - The image is never resident. What this package holds while it runs is one
//     buffer plus whatever the decoder needs for the stream, which is a property
//     of the images the publisher serves rather than of the code reading them:
//     measured against the publisher's own RPi5 image, 1.1 GiB decompressed
//     against roughly 120 MiB of live heap. The claim is therefore "the image
//     was never held", which is what the bounded-memory property is for, and
//     not "bounded by the buffer", which is what the buffer alone is.
//   - A partial result is never at the output path. The bytes are written to a
//     temporary name beside it and renamed onto it once the stream has ended
//     where the format says it should.
//
// What this package cannot claim is said rather than implied; see Limitation.
//
// Decompression reports no progress of its own. The compressed size is the one
// number available and it says nothing about the decompressed one, so any
// fraction shown would be invented. A run's progress reporting is the
// downloader's, where the publisher declares a length.
//
// Nothing here imports an interface package, and nothing here knows what a
// device is. It is given the path of a file that has already arrived and the
// path to write beside it, which is what lets every behaviour in here be
// exercised against a fixture with no network, no terminal, and no 200 MiB
// download.
package decompress
