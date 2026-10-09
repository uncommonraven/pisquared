// Package signature verifies a downloaded image against the detached signature
// its publisher serves beside it, using the publisher's signing key.
//
// This is section 5 of openspec/changes/v0-1-image-download. It is the second of
// the two verifications design.md specifies, and it exists for the threat the
// first one cannot cover: an image server that substitutes an image *and* a
// checksum matching it. A substituted image would still need a signature from a
// key that server does not control.
//
// Three properties in design.md are what this package is:
//
//   - The key is retrieved at verification time, by key ID, from a public
//     keyserver. That is what the publisher's own imaging script does, and it is
//     what makes a key rotation a non-event here rather than a change somebody
//     has to notice. The cost design.md records as accepted is real and is stated
//     in the result rather than left for a reader to assume: the keyserver is a
//     third party, and the key is identified by 64 bits rather than by a
//     fingerprint a reader could check independently.
//   - The key is held in a temporary directory rather than in the operator's own
//     keyring, so running this tool leaves nothing behind on their machine.
//   - A signature that could not be checked is reported as unverified rather
//     than as a pass, and never as a bad signature. "Nobody could tell us" and
//     "the answer was no" are different facts, and an operator acting on the
//     second one needs the first.
//
// Three outcomes, and none of them is silently another:
//
//   - Signed: the signature verified, and the key that made it is named.
//   - BadSignature: it did not, returned as the error that halts a run.
//   - Unverified: the key or the signature could not be retrieved, with the
//     reason kept.
//
// Nothing here imports an interface package, and nothing here knows what a device
// is. It is given an address and the path of a file that has already arrived,
// which is what lets every behaviour below be exercised against a local keyserver
// and a local publisher with no network, no terminal, and no 200 MiB fixture.
package signature
