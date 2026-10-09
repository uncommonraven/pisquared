// Package deps proves, on evidence, that the third-party libraries this version
// depends on do the things the sections that follow require of them.
//
// Section 2 of openspec/changes/v0-1-image-download exists because a dependency
// that cannot stream, cannot be tested without a terminal, or breaks the static
// build is far cheaper to find here than three sections later. Each file here is
// the proof for one such dependency; none of them is the feature itself.
//
// The features land elsewhere. Section 9 builds the interface, section 7 builds
// decompression, and section 6 builds signature verification, and each of those
// is written against what these files establish rather than against what they
// happen to contain.
//
// Nothing in here is imported by cmd/pisquared. That is deliberate: the point of
// the proofs is that they can be read and run on their own.
package deps
