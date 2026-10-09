package catalogue

import (
	"errors"
	"regexp"
	"strings"
	"testing"
)

// The tests in this file cover the committed table and the resolution over it:
// that it carries exactly the devices design.md's table names, that each
// resolves to the image design.md says it does, that the addresses are the
// shape the publisher actually serves, and that an unlisted device gets nothing
// rather than something plausible.
//
// The network check lives in check_test.go.

// designDevices is the device list from design.md's table of images, written
// out again here rather than read out of the catalogue itself.
//
// It is written independently on purpose. A test that derived its expectation
// from the data it is checking would agree with whatever the catalogue happened
// to hold, including a device that does not exist and the omission of one that
// does. This is the statement the catalogue is measured against.
var designDevices = []string{
	"Raspberry Pi 1 A/B/B+",
	"Raspberry Pi Zero",
	"Raspberry Pi Zero W",
	"Compute Module 1",
	"Raspberry Pi 2 (PCB v1.1)",
	"Raspberry Pi 2 (PCB v1.2)",
	"Raspberry Pi 3 A/B/B+",
	"Raspberry Pi 4",
	"Raspberry Pi 400",
	"Compute Module 4",
	"Raspberry Pi Zero 2 W",
	"Raspberry Pi 5",
	"Raspberry Pi 500",
	"Compute Module 5",
}

// TestCatalogueCarriesEveryDeviceDesignNamesExactlyOnce is task 3.1's check.
// Every device in design.md's table is present, none is present twice, and
// nothing else is — so a rename cannot quietly drop a board and a copy-paste
// cannot quietly add one.
func TestCatalogueCarriesEveryDeviceDesignNamesExactlyOnce(t *testing.T) {
	counts := make(map[string]int, len(Devices))
	for _, d := range Devices {
		if d.Name == "" {
			t.Errorf("the device with id %q has no name", d.ID)
		}
		counts[d.Name]++
	}

	for _, name := range designDevices {
		switch counts[name] {
		case 1:
		case 0:
			t.Errorf("design.md's table names %q and the catalogue does not carry it", name)
		default:
			t.Errorf("the catalogue carries %q %d times; design.md's table names it once", name, counts[name])
		}
		delete(counts, name)
	}

	for name, count := range counts {
		t.Errorf("the catalogue carries %q %d times and design.md's table does not name it", name, count)
	}
}

// TestEveryCatalogueEntryCarriesItsAddressAndCaveat is the rest of task 3.1.
// An entry with no caveat is legitimate — most entries have none — but an entry
// whose image design.md attaches a caveat to must carry that caveat, and no
// other entry may carry it.
func TestEveryCatalogueEntryCarriesItsAddressAndCaveat(t *testing.T) {
	// The entry design.md gives the PCB caveat to, by name. Every entry
	// resolving to the RPi234 image carries the other one, and that list is
	// built from the board rather than written out, because the claim under
	// test is precisely that every RPi234 entry carries it.
	wanted := map[string]string{
		"Raspberry Pi 2 (PCB v1.1)": CaveatRPi2PCB,
	}
	const rpi234Caveat = CaveatRPi234Pi5

	for _, d := range Devices {
		if d.Address() == "" {
			t.Errorf("%s (%s) carries no address", d.Name, d.ID)
		}
		if d.Address() != AddressPrefix+d.ImageName() {
			t.Errorf("%s (%s) address %q is not its image name at the publisher's address",
				d.Name, d.ID, d.Address())
		}

		want := wanted[d.Name]
		if d.Board == "RPi234" {
			want = rpi234Caveat
		}

		switch {
		case want == "" && len(d.Caveats) > 0:
			t.Errorf("%s (%s) carries %d caveats; design.md attaches none to it", d.Name, d.ID, len(d.Caveats))
		case want != "" && !carries(d.Caveats, want):
			t.Errorf("%s (%s) does not carry the caveat design.md attaches to it:\n  %s", d.Name, d.ID, want)
		}
	}

	// The caveats must not have been attached to the wrong images, which is the
	// failure a per-entry list of caveats can have and a shared constant cannot.
	for _, d := range Devices {
		if d.Board != "RPi234" && carries(d.Caveats, rpi234Caveat) {
			t.Errorf("%s (%s) resolves to %s and carries the caveat about the RPi234 image",
				d.Name, d.ID, d.ImageName())
		}
		if d.Board != "RPi2" && carries(d.Caveats, CaveatRPi2PCB) {
			t.Errorf("%s (%s) resolves to %s and carries the caveat about the RPi2 image",
				d.Name, d.ID, d.ImageName())
		}
	}
}

func carries(have []string, want string) bool {
	for _, h := range have {
		if h == want {
			return true
		}
	}
	return false
}

// TestDeviceIdentifiersAreUniqueAndResolvable is the table's own integrity: a
// duplicate identifier would make Resolve answer with whichever entry came
// first, which is a silent choice between two devices.
func TestDeviceIdentifiersAreUniqueAndResolvable(t *testing.T) {
	seen := make(map[string]string, len(Devices))
	for _, d := range Devices {
		if d.ID == "" {
			t.Errorf("%s has no identifier", d.Name)
			continue
		}
		if strings.ContainsAny(d.ID, " /\\") {
			t.Errorf("the identifier %q contains a character that would be awkward to type or write to a flag", d.ID)
		}
		if previous, taken := seen[d.ID]; taken {
			t.Errorf("the identifier %q is used by both %s and %s", d.ID, previous, d.Name)
		}
		seen[d.ID] = d.Name

		res, err := Resolve(d.ID)
		if err != nil {
			t.Errorf("Resolve(%q) for %s: %v", d.ID, d.Name, err)
			continue
		}
		if res.Device.ID != d.ID {
			t.Errorf("Resolve(%q) returned the device %q", d.ID, res.Device.ID)
		}
		if res.Address != d.Address() {
			t.Errorf("Resolve(%q) returned %q, want %q", d.ID, res.Address, d.Address())
		}
	}
}

// TestDevicesSharingAnImageResolveToThatImage is task 3.2's first half. A Pi 4,
// a Pi 400, and a Compute Module 4 are one image, and the tool should say they
// share it rather than presenting three similar ones.
func TestDevicesSharingAnImageResolveToThatImage(t *testing.T) {
	const want = AddressPrefix + "DietPi_RPi234-ARMv8-Trixie.img.xz"

	first, err := Resolve("rpi-4")
	if err != nil {
		t.Fatalf("Resolve(rpi-4): %v", err)
	}
	if first.Address != want {
		t.Errorf("Resolve(rpi-4) = %q, want %q", first.Address, want)
	}

	for _, id := range []string{"rpi-400", "cm-4"} {
		res, err := Resolve(id)
		if err != nil {
			t.Fatalf("Resolve(%q): %v", id, err)
		}
		if res.Address != want {
			t.Errorf("Resolve(%q) = %q, want %q", id, res.Address, want)
		}
	}
}

// TestAPi5ResolvesToTheRPi5ImageNotTheOneThatMerelyBootsIt is task 3.2's
// second half. The RPi234 image boots a Pi 5, and upstream nonetheless
// recommends RPi5 for it. Resolving a Pi 5 to RPi234 would be applying that
// preference silently, which design.md asks to be shown rather than done.
func TestAPi5ResolvesToTheRPi5ImageNotTheOneThatMerelyBootsIt(t *testing.T) {
	const want = AddressPrefix + "DietPi_RPi5-ARMv8-Trixie.img.xz"

	res, err := Resolve("rpi-5")
	if err != nil {
		t.Fatalf("Resolve(rpi-5): %v", err)
	}
	if res.Address != want {
		t.Errorf("Resolve(rpi-5) = %q, want %q", res.Address, want)
	}
	if strings.Contains(res.Address, "RPi234") {
		t.Errorf("Resolve(rpi-5) returned %q, which is the image that merely also boots it", res.Address)
	}

	// The other two devices upstream puts on this image, so that fixing one
	// entry cannot satisfy the assertion above.
	for _, id := range []string{"rpi-500", "cm-5"} {
		other, err := Resolve(id)
		if err != nil {
			t.Fatalf("Resolve(%q): %v", id, err)
		}
		if other.Address != want {
			t.Errorf("Resolve(%q) = %q, want %q", id, other.Address, want)
		}
	}
}

// addressShape is the form the publisher's image addresses take, from the
// composition in .build/images/dietpi-build line 258 and the files served at
// AddressPrefix.
var addressShape = regexp.MustCompile(`^https://dietpi\.com/downloads/images/DietPi_[A-Za-z0-9]+-[A-Za-z0-9]+-[A-Za-z0-9]+\.img\.xz$`)

// TestEveryAddressHasTheShapeThePublisherServes is task 3.3. A catalogue entry
// with a mistyped board or a wrong architecture is a 404 the operator finds
// mid-download; catching it here makes it a failed test instead.
func TestEveryAddressHasTheShapeThePublisherServes(t *testing.T) {
	if !addressShape.MatchString(AddressPrefix + "DietPi_RPi234-ARMv8-Trixie.img.xz") {
		t.Fatal("the address shape rejects a real address, so this test would prove nothing")
	}

	for _, d := range Devices {
		if !addressShape.MatchString(d.Address()) {
			t.Errorf("%s (%s) address %q does not have the shape "+
				"https://dietpi.com/downloads/images/DietPi_<board>-<arch>-<codename>.img.xz",
				d.Name, d.ID, d.Address())
		}
	}
}

// TestTheAddressShapeRejectsMalformedEntries is the other half of task 3.3. A
// test that cannot fail is not a check, so the shape is run against addresses
// built from entries that are wrong in each of the ways an entry can be wrong.
func TestTheAddressShapeRejectsMalformedEntries(t *testing.T) {
	for _, tc := range []struct {
		name string
		bad  Device
		want string
	}{
		{"a space in the board", Device{Board: "RPi 234", Arch: "ARMv8", Codename: "Trixie"},
			AddressPrefix + "DietPi_RPi 234-ARMv8-Trixie.img.xz"},
		{"a missing architecture", Device{Board: "RPi234", Codename: "Trixie"},
			AddressPrefix + "DietPi_RPi234--Trixie.img.xz"},
		{"a missing codename", Device{Board: "RPi234", Arch: "ARMv8"},
			AddressPrefix + "DietPi_RPi234-ARMv8-.img.xz"},
		{"a path separator in the board", Device{Board: "../../etc/passwd", Arch: "ARMv8", Codename: "Trixie"},
			AddressPrefix + "DietPi_../../etc/passwd-ARMv8-Trixie.img.xz"},
		{"an extra component", Device{Board: "RPi234", Arch: "ARMv8", Codename: "Trixie-arm64"},
			AddressPrefix + "DietPi_RPi234-ARMv8-Trixie-arm64.img.xz"},
		{"nothing at all", Device{}, AddressPrefix + "DietPi_--.img.xz"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if got := tc.bad.Address(); got != tc.want {
				t.Fatalf("Address() = %q, want %q", got, tc.want)
			}
			if addressShape.MatchString(tc.bad.Address()) {
				t.Errorf("the address shape accepted %q, so the test over the real catalogue proves nothing",
					tc.bad.Address())
			}
		})
	}
}

// TestTheAddressShapeRejectsAddressesThatAreNotImages covers the ways an
// address can be the wrong shape without any catalogue field being able to
// produce it. Device.Address always builds at AddressPrefix, so these are
// checked as literal addresses: they are the mistakes that reach this package
// from elsewhere — a copied address on the documentation page, or a mirror
// someone has pointed at — and section 4's drift check compares against
// exactly this shape.
func TestTheAddressShapeRejectsAddressesThatAreNotImages(t *testing.T) {
	for _, address := range []string{
		"http://dietpi.com/downloads/images/DietPi_RPi234-ARMv8-Trixie.img.xz",
		"https://dietpi.example.invalid/downloads/images/DietPi_RPi234-ARMv8-Trixie.img.xz",
		"https://dietpi.com/downloads/images/DietPi_RPi234-ARMv8-Trixie.img.xz.sha256",
		"https://dietpi.com/downloads/images/DietPi_RPi234-ARMv8-Trixie.img.xz.asc",
		"https://dietpi.com/images/DietPi_RPi234-ARMv8-Trixie.img.xz",
		"https://github.com/MichaIng/DietPi/releases/download/v10.7/DietPi_RPi234-ARMv8-Trixie.img.xz",
		"DietPi_RPi234-ARMv8-Trixie.img.xz",
		"https://dietpi.com/downloads/images/",
		"",
	} {
		if addressShape.MatchString(address) {
			t.Errorf("the address shape accepted %q, which is not an image address at %s", address, AddressPrefix)
		}
	}
}

// TestAnUnlistedDeviceResolvesToNoImageAndNoDefault is task 3.4, and the one
// this package exists for.
//
// The identifiers below are near misses and plausible typos rather than random
// strings, because the failure being guarded against is not someone typing
// nonsense: it is someone typing a device that exists but is not in this
// table, or typing one that is nearly one that is. Every one of them must come
// back with nothing at all, because a wrong image is discovered only after a
// card has been erased.
func TestAnUnlistedDeviceResolvesToNoImageAndNoDefault(t *testing.T) {
	for _, id := range []string{
		"",
		"pico",
		"rpi-pico",
		"rpi-6",
		"rpi-50",
		"rpi-5 ",
		" rpi-5",
		"RPI-5",
		"Raspberry Pi 5",
		"rpi_5",
		"rpi-4b",
		"cm-40",
		"cm5",
		"zero-2",
		"esp32",
		"orangepi-5",
	} {
		res, err := Resolve(id)
		if err == nil {
			t.Errorf("Resolve(%q) succeeded with %q; a device that is not in the catalogue must resolve to nothing",
				id, res.Address)
			continue
		}
		if res.Address != "" {
			t.Errorf("Resolve(%q) returned an error and the address %q anyway; there must be no path "+
				"by which an unlisted device is handed an image", id, res.Address)
		}
		if len(res.Caveats) > 0 {
			t.Errorf("Resolve(%q) returned caveats for an unlisted device: %v", id, res.Caveats)
		}
		if res.Device.ID != "" || res.Device.Name != "" {
			t.Errorf("Resolve(%q) returned the device %q/%q; it must carry none", id, res.Device.ID, res.Device.Name)
		}

		var notListed *NotListed
		if !errors.As(err, &notListed) {
			t.Errorf("Resolve(%q) returned %T, want *NotListed", id, err)
			continue
		}
		if notListed.ID != id {
			t.Errorf("Resolve(%q) reported %q as the identifier it could not find", id, notListed.ID)
		}
	}
}

// TestAnUnlistedDeviceIsToldHowToProceed is the other half of task 3.4: the
// absence of a default is only useful if the operator is given somewhere to go
// instead. The message has to be actionable on its own, since it is what the
// selection view and the non-interactive mode both print.
func TestAnUnlistedDeviceIsToldHowToProceed(t *testing.T) {
	_, err := Resolve("rpi-pico")
	if err == nil {
		t.Fatal("Resolve(rpi-pico) succeeded")
	}

	message := err.Error()
	for _, want := range []string{
		"rpi-pico",      // what was asked for
		"Revision",      // how to identify the board
		"/proc/cpuinfo", // where to read it
		AddressPrefix,   // where the publisher lists its images
		ManualProcedure, // how to proceed without this tool
	} {
		if !strings.Contains(message, want) {
			t.Errorf("the message does not mention %q, so it leaves the operator nothing to do:\n%s", want, message)
		}
	}

	if len(message) < 120 {
		t.Errorf("the message is %d characters, too short to tell an operator what to do next", len(message))
	}
}

// TestResolvingAnUnlistedDeviceNeverLeaksACatalogueAddress closes the loophole
// in the test above. Checking a fixed list of bad identifiers shows that those
// particular ones resolve to nothing; this walks near misses of every real
// identifier and shows that none of them yields an image either.
func TestResolvingAnUnlistedDeviceNeverLeaksACatalogueAddress(t *testing.T) {
	known := make(map[string]bool, len(Devices))
	for _, d := range Devices {
		known[d.Address()] = true
	}

	var probes int
	for _, d := range Devices {
		for _, id := range []string{
			d.ID + "x",
			"x" + d.ID,
			d.ID + " ",
			d.ID[:len(d.ID)-1],
			d.Name,
			d.Board,
		} {
			res, err := Resolve(id)
			if err != nil && res.Address != "" {
				t.Errorf("Resolve(%q) returned %q alongside its error", id, res.Address)
			}
			if err == nil && !known[res.Address] {
				t.Errorf("Resolve(%q) returned %q, which is not an address in the catalogue", id, res.Address)
			}
			probes++
		}
	}
	t.Logf("probed %d near misses across %d devices", probes, len(Devices))
}
