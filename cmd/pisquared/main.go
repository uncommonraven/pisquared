// Command pisquared obtains a verified, flashable base image for the device an
// operator names, and records what it was verified against beside the result.
//
// This file is the repository scaffold for v0.1. None of the acquisition steps
// it names exist yet: there is no device catalogue, no download, no
// verification, and no decompression. The binary therefore refuses to run
// rather than pretending to produce an image it cannot produce.
//
// The work planned for it is in ROADMAP.md under v0.1, and the steps this
// version will perform are specified in openspec/changes/v0-1-image-download.
//
// Everything below this package is logic that runs and is tested with no
// terminal attached. Nothing that decides anything belongs in a command file.
package main

import (
	"fmt"
	"os"
)

func main() {
	fmt.Fprintln(os.Stderr, "pisquared: obtaining an image is not implemented yet; see ROADMAP.md v0.1")
	os.Exit(1)
}
