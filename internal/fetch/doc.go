// Package fetch obtains a published image file and verifies it against the
// checksum its publisher publishes for it.
//
// This is section 4 of openspec/changes/v0-1-image-download. Three properties
// in design.md's "Everything streams, because the image is larger than the
// memory it comes from" are what this package is:
//
//   - The transfer streams to disk and hashes as it goes, so peak memory is
//     bounded by the buffer rather than by the image. Roughly 200 MiB compressed
//     arrives and 1.1 GiB leaves this version, and neither is ever resident.
//   - The bytes land at a temporary name beside the output and are renamed onto
//     it only once every byte has arrived and the checksum has been consulted.
//     That is what makes an interrupted transfer distinguishable from a
//     finished image without asking the operator, which the specification
//     requires of an interrupted, cancelled, and failed download alike.
//   - An interrupted transfer is not continued: a retry starts over, which the
//     specification permits and which is what makes the verified result
//     identical either way rather than dependent on the order the bytes
//     arrived in. Resumption was considered and not built; Download records why.
//
// Three states are reported and none of them is silently another:
//
//   - Verified, the observed digest equalled the published one.
//   - Mismatch, and the temporary file is removed so nothing that failed can be
//     mistaken for something to retry from.
//   - Unverified, the published checksum could not be retrieved, with the reason
//     kept. It is never reported as verified and never as a mismatch, because
//     "nobody could tell us the answer" and "the answer was different" are
//     different facts and an operator acting on the second one needs the first.
//
// Nothing here imports an interface package, and nothing here knows what a
// device is. The catalogue resolves a device to an address; this package is
// given an address and an output path, which is what lets every behaviour below
// be exercised against a local server with no network, no terminal, and no
// 200 MiB fixture.
package fetch
