// Package catalogue is the committed table mapping a device an operator owns to
// the image that boots it.
//
// design.md gives the reason it is committed rather than derived: the publisher
// serves an nginx autoindex of filenames and nothing that says which device
// each one boots. Deciding that RPi234 boots a Compute Module 4 is knowledge,
// and it is knowledge that belongs in version control where a reviewer can
// watch it change, rather than in a heuristic that guesses while the operator
// waits.
//
// The table has no default and no nearest match. A device it does not carry
// resolves to no image at all and returns what to tell the operator instead,
// because handing an unlisted device some other device's image is exactly the
// failure this package exists to prevent — and per the proposal it is
// discovered only after a card has been erased.
//
// Nothing here reaches the network. Check is the one function in the package
// that does, and the build is what calls it; see check.go.
package catalogue

import "fmt"

// AddressPrefix is where the publisher serves every image, and where every
// address in the table is built. There is no machine-readable index to consult
// instead: design.md records that the GitHub releases API returns an empty
// assets array for every recent version, so this pattern and the three parts
// below it are the whole of what upstream tells us about its own filenames.
const AddressPrefix = "https://dietpi.com/downloads/images/"

// ManualProcedure is where the obtaining procedure is documented, written as
// though this tool did not exist. An operator whose device is not in the table
// is sent there, because that page is what they can still follow without this
// repository.
const ManualProcedure = "docs/obtaining-a-verified-image.md"

// The two caveats design.md attaches to this table, written out once each.
//
// The RPi234 caveat is attached to every device that resolves to that image
// rather than to the Pi 5, because that is where an operator meets it: someone
// who owns a Pi 4 is looking at the RPi234 image and should be told it also
// boots a Pi 5, and that upstream recommends a different one for it. Keeping
// the text in one place means the eight devices sharing that image cannot
// drift apart on what it says.
const (
	// CaveatRPi234Pi5 attaches to every device resolving to the RPi234 image.
	// That image does boot a Raspberry Pi 5, and upstream nonetheless
	// recommends RPi5 for those models because it ships a kernel optimized for
	// them with a 16k page size. Resolve already sends a Pi 5 to RPi5 — this
	// is the operator being shown why that was a choice rather than the tool
	// applying a preference silently, which is what design.md asks for.
	CaveatRPi234Pi5 = "This image also boots a Raspberry Pi 5, though upstream recommends the " +
		"RPi5 image for that model: its kernel is built for those machines with a 16k page " +
		"size. If the device you have is a Raspberry Pi 5, choose Raspberry Pi 5 from the " +
		"list, which resolves to that image instead."

	// CaveatRPi2PCB attaches to the Raspberry Pi 2 PCB v1.1, the only board
	// the RPi2 image is built for. A v1.2 board has a different processor and
	// is listed separately, because design.md's table distinguishes them and
	// the image is not interchangeable between them.
	CaveatRPi2PCB = "The RPi2 image is built for the Raspberry Pi 2 PCB v1.1 only. A PCB v1.2 " +
		"board has a different processor and is listed separately as Raspberry Pi 2 (PCB " +
		"v1.2), which resolves to the 64-bit RPi234 image. Flashing this image to a v1.2 " +
		"board produces a host that does not boot."
)

// Device is one entry in the catalogue: a device family, the publisher image
// that boots it, the architecture that image is built for, and the caveats that
// travel with it.
type Device struct {
	// ID is the stable key a choice is resolved by. It is a slug rather than
	// the display name so it can be typed on a command line, carried as a flag
	// by the non-interactive mode, and written into the record beside the
	// image, without the spelling of a product name changing underneath any
	// of those.
	ID string

	// Name is what the device is called in the selection view and in the
	// record. Devices that share one image are one entry rather than several:
	// they are one choice, not similar ones, and listing them apart would
	// imply a distinction the publisher does not make.
	Name string

	// Board, Arch, and Codename are the three parts of the publisher's image
	// filename. They are committed separately so that a rename upstream shows
	// up here as a diff on the field that moved rather than hidden inside an
	// opaque string, and so the address test can assert the shape upstream
	// actually serves rather than only that the string is stable.
	Board    string
	Arch     string
	Codename string

	// Caveats are shown whenever the device resolves. They are empty where the
	// image this device resolves to is the one upstream recommends for it and
	// there is nothing worth saying about it.
	Caveats []string
}

// Devices is the catalogue: every device family this tool will obtain an image
// for, the image that boots it, the architecture that image is built for, and
// the caveats that travel with it.
//
// The four images and the devices each covers are design.md's table, verified
// against the publisher's directory on 2026-10-09. RP2040-class boards such as
// the Pico are absent deliberately and not by oversight: DietPi is a
// Debian-based distribution and cannot run on a microcontroller, so there is no
// image to resolve them to. Upstream adding a board this table does not name is
// a failure the build-time check cannot catch, which design.md records as
// accepted rather than solved, and which is why the manual procedure stands on
// its own.
//
// The order is the selection view's order, oldest board to newest, so the list
// reads as a progression rather than as an alphabetical accident.
var Devices = []Device{
	{
		ID:       "rpi-1",
		Name:     "Raspberry Pi 1 A/B/B+",
		Board:    "RPi1",
		Arch:     "ARMv6",
		Codename: "Trixie",
	},
	{
		ID:       "zero",
		Name:     "Raspberry Pi Zero",
		Board:    "RPi1",
		Arch:     "ARMv6",
		Codename: "Trixie",
	},
	{
		ID:       "zero-w",
		Name:     "Raspberry Pi Zero W",
		Board:    "RPi1",
		Arch:     "ARMv6",
		Codename: "Trixie",
	},
	{
		ID:       "cm-1",
		Name:     "Compute Module 1",
		Board:    "RPi1",
		Arch:     "ARMv6",
		Codename: "Trixie",
	},
	{
		ID:       "rpi-2-v1",
		Name:     "Raspberry Pi 2 (PCB v1.1)",
		Board:    "RPi2",
		Arch:     "ARMv7",
		Codename: "Trixie",
		Caveats:  []string{CaveatRPi2PCB},
	},
	{
		ID:       "rpi-2-v2",
		Name:     "Raspberry Pi 2 (PCB v1.2)",
		Board:    "RPi234",
		Arch:     "ARMv8",
		Codename: "Trixie",
		Caveats:  []string{CaveatRPi234Pi5},
	},
	{
		ID:       "rpi-3",
		Name:     "Raspberry Pi 3 A/B/B+",
		Board:    "RPi234",
		Arch:     "ARMv8",
		Codename: "Trixie",
		Caveats:  []string{CaveatRPi234Pi5},
	},
	{
		ID:       "rpi-4",
		Name:     "Raspberry Pi 4",
		Board:    "RPi234",
		Arch:     "ARMv8",
		Codename: "Trixie",
		Caveats:  []string{CaveatRPi234Pi5},
	},
	{
		ID:       "rpi-400",
		Name:     "Raspberry Pi 400",
		Board:    "RPi234",
		Arch:     "ARMv8",
		Codename: "Trixie",
		Caveats:  []string{CaveatRPi234Pi5},
	},
	{
		ID:       "cm-4",
		Name:     "Compute Module 4",
		Board:    "RPi234",
		Arch:     "ARMv8",
		Codename: "Trixie",
		Caveats:  []string{CaveatRPi234Pi5},
	},
	{
		ID:       "zero-2-w",
		Name:     "Raspberry Pi Zero 2 W",
		Board:    "RPi234",
		Arch:     "ARMv8",
		Codename: "Trixie",
		Caveats:  []string{CaveatRPi234Pi5},
	},
	{
		ID:       "rpi-5",
		Name:     "Raspberry Pi 5",
		Board:    "RPi5",
		Arch:     "ARMv8",
		Codename: "Trixie",
	},
	{
		ID:       "rpi-500",
		Name:     "Raspberry Pi 500",
		Board:    "RPi5",
		Arch:     "ARMv8",
		Codename: "Trixie",
	},
	{
		ID:       "cm-5",
		Name:     "Compute Module 5",
		Board:    "RPi5",
		Arch:     "ARMv8",
		Codename: "Trixie",
	},
}

// Resolution is what a device was resolved to: the device as the table carries
// it, the address to fetch from, and the caveats to show about it.
type Resolution struct {
	// Device is the catalogue entry the identifier named.
	Device Device

	// Address is the permalink to fetch. It is empty in the zero Resolution,
	// which is the one Resolve ever returns alongside an error.
	Address string

	// Caveats is copied from the device rather than left to be reached through
	// it, so a caller that renders a resolution cannot accidentally present an
	// image without the caveat that travels with it.
	Caveats []string
}

// Resolve maps the identifier of a device the operator named to the image that
// boots it.
//
// An identifier the table does not carry is a *NotListed error and a zero
// Resolution: no address, no caveats, and nothing to suggest another device
// was meant. Nearest-match lookup was considered and rejected. It would turn
// "cm-4" into "cm-40" and "rpi-5" into "rpi-50" without saying so, and the cost
// of that guess is a card erased and a host that does not boot. An unlisted
// device is an inconvenience; a wrong image is a destroyed card.
func Resolve(id string) (Resolution, error) {
	for _, d := range Devices {
		if d.ID != id {
			continue
		}
		return Resolution{Device: d, Address: d.Address(), Caveats: d.Caveats}, nil
	}
	return Resolution{}, &NotListed{ID: id}
}

// NotListed is returned by Resolve for an identifier the catalogue does not
// carry. Its message is what the operator is shown, so it says how to find out
// what their board is called and where to go without this tool, rather than
// only reporting that this table lacks the name they typed.
type NotListed struct {
	// ID is the identifier that was asked for and not found, kept so a caller
	// can record what was typed without parsing the message back apart.
	ID string
}

func (e *NotListed) Error() string {
	return fmt.Sprintf("no image is chosen for %q", e.ID) +
		": which image boots which device is knowledge this project commits rather than derives, " +
		"so a device that is not in the table is not guessed at. Find the board's name — the " +
		"Revision line of /proc/cpuinfo on a machine already booted with one, or the model " +
		"printed on the box — and look it up in the publisher's own directory at " + AddressPrefix +
		", which lists the image filenames. Obtaining an image without this tool is documented in " +
		ManualProcedure + "."
}

// ImageName is the publisher's filename for the device's image, before it is
// placed at the publisher's address.
func (d Device) ImageName() string {
	return "DietPi_" + d.Board + "-" + d.Arch + "-" + d.Codename + ".img.xz"
}

// Address is the permalink that boots the device.
//
// It carries no version, and that is the design rather than an omission. From
// .build/images/dietpi-build line 258 the publisher composes the name as
// DietPi_$iname-$iarch-${distro^} and serves it permanently, so this address is
// always the current stable build and cannot be stale by claiming to be
// current. A version is resolved for the record, not to build a URL to fetch.
func (d Device) Address() string {
	return AddressPrefix + d.ImageName()
}
