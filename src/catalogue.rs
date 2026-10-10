//! The committed table mapping a device an operator owns to the image that boots it.
//!
//! The publisher serves an nginx autoindex of filenames and nothing that says which device each one
//! boots. Deciding that RPi234 boots a Compute Module 4 is knowledge, and it belongs in version
//! control where a reviewer can watch it change.

/// Where the publisher serves every image. There is no machine-readable index to consult instead.
pub const ADDRESS_PREFIX: &str = "https://dietpi.com/downloads/images/";

/// Where the obtaining procedure is documented, written as though this tool did not exist. An operator
/// whose device is not in the catalogue is sent there.
pub const MANUAL_PROCEDURE: &str = "docs/obtaining-a-verified-image.md";

/// The RPi234 caveat is attached to every device that resolves to that image, because that is where
/// an operator meets it. That image does boot a Raspberry Pi 5, and upstream nonetheless recommends
/// RPi5 for those models because it ships a kernel optimized for them with a 16k page size.
pub const CAVEAT_RPI234_PI5: &str = concat!(
    "This image also boots a Raspberry Pi 5, though upstream recommends the RPi5 image for that model: ",
    "its kernel is built for those machines with a 16k page size. If the device you have is a Raspberry Pi 5, ",
    "choose Raspberry Pi 5 from the list, which resolves to that image instead."
);

/// The RPi2 image is built for the Raspberry Pi 2 PCB v1.1 only. A v1.2 board has a different
/// processor and is listed separately.
pub const CAVEAT_RPI2_PCB: &str = concat!(
    "The RPi2 image is built for the Raspberry Pi 2 PCB v1.1 only. A PCB v1.2 board has a different ",
    "processor and is listed separately as Raspberry Pi 2 (PCB v1.2), which resolves to the 64-bit RPi234 image. ",
    "Flashing this image to a v1.2 board produces a host that does not boot."
);

/// One entry in the catalogue: a device family, the publisher image that boots it, the architecture
/// that image is built for, and the caveats that travel with it.
#[derive(Debug)]
pub struct Device {
    /// A stable slug the device is chosen by, so it can be typed on a command line.
    pub id: &'static str,
    /// What the device is called in the selection view and in the record.
    pub name: &'static str,
    /// The three parts of the publisher's image filename, kept apart so an upstream rename shows up
    /// as a diff on the field that moved.
    pub board: &'static str,
    pub arch: &'static str,
    pub codename: &'static str,
    /// Shown whenever the device resolves. Empty where upstream's recommended image is the one used.
    pub caveats: &'static [&'static str],
}

impl Device {
    /// The publisher's filename for the device's image.
    pub fn image_name(&self) -> String {
        format!(
            "DietPi_{}-{}-{}.img.xz",
            self.board, self.arch, self.codename
        )
    }

    /// The permalink that boots the device. It carries no version: the publisher serves it as the
    /// current stable build, so it cannot be stale by claiming to be current.
    pub fn address(&self) -> String {
        format!("{ADDRESS_PREFIX}{}", self.image_name())
    }
}

/// The catalogue, in the selection view's order, oldest board to newest. RP2040-class boards such as
/// the Pico are absent deliberately: DietPi is Debian-based and cannot run on a microcontroller.
pub const DEVICES: &[Device] = &[
    Device {
        id: "rpi-1",
        name: "Raspberry Pi 1 A/B/B+",
        board: "RPi1",
        arch: "ARMv6",
        codename: "Trixie",
        caveats: &[],
    },
    Device {
        id: "zero",
        name: "Raspberry Pi Zero",
        board: "RPi1",
        arch: "ARMv6",
        codename: "Trixie",
        caveats: &[],
    },
    Device {
        id: "zero-w",
        name: "Raspberry Pi Zero W",
        board: "RPi1",
        arch: "ARMv6",
        codename: "Trixie",
        caveats: &[],
    },
    Device {
        id: "cm-1",
        name: "Compute Module 1",
        board: "RPi1",
        arch: "ARMv6",
        codename: "Trixie",
        caveats: &[],
    },
    Device {
        id: "rpi-2-v1",
        name: "Raspberry Pi 2 (PCB v1.1)",
        board: "RPi2",
        arch: "ARMv7",
        codename: "Trixie",
        caveats: &[CAVEAT_RPI2_PCB],
    },
    Device {
        id: "rpi-2-v2",
        name: "Raspberry Pi 2 (PCB v1.2)",
        board: "RPi234",
        arch: "ARMv8",
        codename: "Trixie",
        caveats: &[CAVEAT_RPI234_PI5],
    },
    Device {
        id: "rpi-3",
        name: "Raspberry Pi 3 A/B/B+",
        board: "RPi234",
        arch: "ARMv8",
        codename: "Trixie",
        caveats: &[CAVEAT_RPI234_PI5],
    },
    Device {
        id: "rpi-4",
        name: "Raspberry Pi 4",
        board: "RPi234",
        arch: "ARMv8",
        codename: "Trixie",
        caveats: &[CAVEAT_RPI234_PI5],
    },
    Device {
        id: "rpi-400",
        name: "Raspberry Pi 400",
        board: "RPi234",
        arch: "ARMv8",
        codename: "Trixie",
        caveats: &[CAVEAT_RPI234_PI5],
    },
    Device {
        id: "cm-4",
        name: "Compute Module 4",
        board: "RPi234",
        arch: "ARMv8",
        codename: "Trixie",
        caveats: &[CAVEAT_RPI234_PI5],
    },
    Device {
        id: "zero-2-w",
        name: "Raspberry Pi Zero 2 W",
        board: "RPi234",
        arch: "ARMv8",
        codename: "Trixie",
        caveats: &[CAVEAT_RPI234_PI5],
    },
    Device {
        id: "rpi-5",
        name: "Raspberry Pi 5",
        board: "RPi5",
        arch: "ARMv8",
        codename: "Trixie",
        caveats: &[],
    },
    Device {
        id: "rpi-500",
        name: "Raspberry Pi 500",
        board: "RPi5",
        arch: "ARMv8",
        codename: "Trixie",
        caveats: &[],
    },
    Device {
        id: "cm-5",
        name: "Compute Module 5",
        board: "RPi5",
        arch: "ARMv8",
        codename: "Trixie",
        caveats: &[],
    },
];

/// What a device resolves to: the catalogue entry, the address to fetch from, and the caveats to show.
#[derive(Debug)]
pub struct Resolution {
    pub device: &'static Device,
    pub address: String,
    /// Copied from the device, so a resolution cannot be rendered as an image without the caveats that
    /// travel with it.
    pub caveats: &'static [&'static str],
}

/// Maps the identifier of a device the operator named to the image that boots it.
///
/// An identifier the catalogue does not carry is a `NotListed` error and no resolution at all. Nearest-match
/// lookup was rejected: it would turn "cm-4" into "cm-40" and "rpi-5" into "rpi-50" without saying so, and
/// the cost of that guess is an erased card and a host that does not boot.
pub fn resolve(id: &str) -> Result<Resolution, NotListed> {
    DEVICES
        .iter()
        .find(|d| d.id == id)
        .map(|device| Resolution {
            device,
            address: device.address(),
            caveats: device.caveats,
        })
        .ok_or_else(|| NotListed { id: id.to_string() })
}

/// Returned by `resolve` for an identifier the catalogue does not carry. Its message is what the operator is
/// shown, so it says how to find out what their board is called and where to go without this tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotListed {
    /// The identifier that was asked for and not found.
    pub id: String,
}

impl std::fmt::Display for NotListed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "no image is chosen for {:?}: which image boots which device is knowledge this project commits \
             rather than derives, so a device that is not in the table is not guessed at. Find the board's name \
             (the Revision line of /proc/cpuinfo on a machine already booted with one, or the model printed on \
             the box) and look it up in the publisher's own directory at {ADDRESS_PREFIX}, which lists the image \
             filenames. Obtaining an image without this tool is documented in {MANUAL_PROCEDURE}.",
            self.id
        )
    }
}

impl std::error::Error for NotListed {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The device list from design.md's table, written out again rather than read from the catalogue,
    /// so the test measures the catalogue against the design and not against itself.
    const DESIGN_DEVICES: &[&str] = &[
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
    ];

    #[test]
    fn catalogue_carries_every_device_design_names_exactly_once() {
        for name in DESIGN_DEVICES {
            let count = DEVICES.iter().filter(|d| d.name == *name).count();
            assert_eq!(
                count, 1,
                "{name} should appear exactly once in the catalogue"
            );
        }
        assert_eq!(
            DEVICES.len(),
            DESIGN_DEVICES.len(),
            "the catalogue carries devices design.md does not name"
        );
    }

    #[test]
    fn identifiers_are_unique_and_safe_to_type() {
        let mut ids: Vec<&str> = DEVICES.iter().map(|d| d.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), DEVICES.len(), "two devices share an identifier");
        for id in DEVICES.iter().map(|d| d.id) {
            assert!(
                !id.contains([' ', '/', '\\']),
                "{id} is awkward to type or write to a flag"
            );
        }
    }

    #[test]
    fn every_entry_carries_its_address_and_the_caveat_design_attaches_to_it() {
        for d in DEVICES {
            assert_eq!(
                d.address(),
                format!("{ADDRESS_PREFIX}{}", d.image_name()),
                "{}",
                d.name
            );

            let wanted: &[&str] = match d.board {
                "RPi234" => &[CAVEAT_RPI234_PI5],
                "RPi2" => &[CAVEAT_RPI2_PCB],
                _ => &[],
            };
            assert_eq!(d.caveats, wanted, "{} carries the wrong caveats", d.name);
        }
    }

    /// Task 3.2, first half: a Pi 4, a Pi 400, and a Compute Module 4 are one image, and the tool should say
    /// they share it rather than presenting three similar ones.
    #[test]
    fn devices_sharing_an_image_resolve_to_that_image() {
        const WANT: &str = "https://dietpi.com/downloads/images/DietPi_RPi234-ARMv8-Trixie.img.xz";

        for id in ["rpi-4", "rpi-400", "cm-4"] {
            let res = resolve(id).unwrap_or_else(|e| panic!("resolve({id}): {e}"));
            assert_eq!(res.address, WANT, "resolve({id})");
        }
    }

    /// Task 3.2, second half: the RPi234 image boots a Pi 5, and upstream nonetheless recommends RPi5 for
    /// it. Resolving a Pi 5 to RPi234 would apply that preference silently, which design.md asks to be shown
    /// rather than done.
    #[test]
    fn pi5_resolves_to_the_rpi5_image_not_the_one_that_merely_boots_it() {
        const WANT: &str = "https://dietpi.com/downloads/images/DietPi_RPi5-ARMv8-Trixie.img.xz";

        let res = resolve("rpi-5").expect("resolve(rpi-5)");
        assert_eq!(res.address, WANT);
        assert!(!res.address.contains("RPi234"));

        for id in ["rpi-500", "cm-5"] {
            let other = resolve(id).unwrap_or_else(|e| panic!("resolve({id}): {e}"));
            assert_eq!(other.address, WANT, "resolve({id})");
        }
    }

    #[test]
    fn every_identifier_resolves_to_its_own_device_and_address() {
        for d in DEVICES {
            let res = resolve(d.id).unwrap_or_else(|e| panic!("resolve({}): {e}", d.id));
            assert_eq!(res.device.id, d.id);
            assert_eq!(res.address, d.address());
            assert_eq!(res.caveats, d.caveats);
        }
    }

    /// Whether an address is the shape the publisher serves: its images directory, then
    /// `DietPi_<board>-<arch>-<codename>.img.xz`, where each part is a non-empty alphanumeric run. Written
    /// by hand rather than with a regex so the port adds no dependency for it.
    fn is_publisher_image_address(address: &str) -> bool {
        let Some(name) = address.strip_prefix(ADDRESS_PREFIX) else {
            return false;
        };
        let Some(parts) = name
            .strip_prefix("DietPi_")
            .and_then(|rest| rest.strip_suffix(".img.xz"))
        else {
            return false;
        };
        let parts: Vec<&str> = parts.split('-').collect();
        parts.len() == 3
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric()))
    }

    /// Task 3.3: every catalogue address has the shape the publisher serves.
    #[test]
    fn every_address_has_the_shape_the_publisher_serves() {
        assert!(
            is_publisher_image_address(
                "https://dietpi.com/downloads/images/DietPi_RPi234-ARMv8-Trixie.img.xz"
            ),
            "the address shape rejects a real address, so this test would prove nothing"
        );

        for d in DEVICES {
            assert!(
                is_publisher_image_address(&d.address()),
                "{} ({}) address {:?} does not have the shape \
                 https://dietpi.com/downloads/images/DietPi_<board>-<arch>-<codename>.img.xz",
                d.name,
                d.id,
                d.address()
            );
        }
    }

    /// Task 3.3, the other half: a test that cannot fail is not a check, so the shape is run against
    /// entries that are wrong in each of the ways an entry can be wrong.
    #[test]
    fn the_address_shape_rejects_malformed_entries() {
        let malformed = [
            (
                "a space in the board",
                device_with("RPi 234", "ARMv8", "Trixie"),
            ),
            (
                "a missing architecture",
                device_with("RPi234", "", "Trixie"),
            ),
            ("a missing codename", device_with("RPi234", "ARMv8", "")),
            (
                "a path separator in the board",
                device_with("../../etc/passwd", "ARMv8", "Trixie"),
            ),
            (
                "an extra component",
                device_with("RPi234", "ARMv8", "Trixie-arm64"),
            ),
            ("nothing at all", device_with("", "", "")),
        ];

        for (case, device) in &malformed {
            assert!(
                !is_publisher_image_address(&device.address()),
                "the address shape accepted {} ({:?}), so the test over the real catalogue proves nothing",
                case,
                device.address()
            );
        }
    }

    /// Task 3.3: the shape also rejects addresses that did not come from a catalogue field, such as a copied
    /// link or a mirror someone has pointed at.
    #[test]
    fn the_address_shape_rejects_addresses_that_are_not_images() {
        for address in [
            "http://dietpi.com/downloads/images/DietPi_RPi234-ARMv8-Trixie.img.xz",
            "https://dietpi.example.invalid/downloads/images/DietPi_RPi234-ARMv8-Trixie.img.xz",
            "https://dietpi.com/downloads/images/DietPi_RPi234-ARMv8-Trixie.img.xz.sha256",
            "https://dietpi.com/downloads/images/DietPi_RPi234-ARMv8-Trixie.img.xz.asc",
            "https://dietpi.com/images/DietPi_RPi234-ARMv8-Trixie.img.xz",
            "https://github.com/MichaIng/DietPi/releases/download/v10.7/DietPi_RPi234-ARMv8-Trixie.img.xz",
            "DietPi_RPi234-ARMv8-Trixie.img.xz",
            "https://dietpi.com/downloads/images/",
            "",
        ] {
            assert!(
                !is_publisher_image_address(address),
                "the address shape accepted {address:?}, which is not an image address at {ADDRESS_PREFIX}"
            );
        }
    }

    fn device_with(board: &'static str, arch: &'static str, codename: &'static str) -> Device {
        Device {
            id: "test",
            name: "test",
            board,
            arch,
            codename,
            caveats: &[],
        }
    }

    /// Task 3.4, the one this module exists for: an identifier the catalogue does not carry resolves to no
    /// image and no default. The identifiers are near misses and plausible typos, because the failure guarded
    /// against is typing a device that exists but is not in this table, or one nearly in it.
    #[test]
    fn an_unlisted_device_resolves_to_no_image_and_no_default() {
        for id in [
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
        ] {
            match resolve(id) {
                Ok(res) => panic!(
                    "resolve({id:?}) succeeded with {:?}; a device not in the catalogue must resolve to nothing",
                    res.address
                ),
                Err(err) => assert_eq!(err.id, id, "resolve({id:?}) reported the wrong identifier"),
            }
        }
    }

    /// Task 3.4, the other half: the absence of a default is only useful if the operator is told where to go
    /// instead. The message is what the selection view and the non-interactive mode both print.
    #[test]
    fn an_unlisted_device_is_told_how_to_proceed() {
        let err = resolve("rpi-pico").expect_err("resolve(rpi-pico) succeeded");
        let message = err.to_string();

        for want in [
            "rpi-pico",
            "Revision",
            "/proc/cpuinfo",
            ADDRESS_PREFIX,
            MANUAL_PROCEDURE,
        ] {
            assert!(
                message.contains(want),
                "the message does not mention {want:?}, so it leaves the operator nothing to do:\n{message}"
            );
        }
        assert!(
            message.len() >= 120,
            "the message is {} characters, too short to tell an operator what to do next",
            message.len()
        );
    }

    /// Task 3.4: near misses of every real identifier must not yield a catalogue address either.
    #[test]
    fn near_misses_of_a_real_identifier_never_yield_a_catalogue_address() {
        let known: Vec<String> = DEVICES.iter().map(Device::address).collect();

        for d in DEVICES {
            let probes = [
                format!("{}x", d.id),
                format!("x{}", d.id),
                format!("{} ", d.id),
                d.id[..d.id.len() - 1].to_string(),
                d.name.to_string(),
                d.board.to_string(),
            ];
            for probe in &probes {
                if let Ok(res) = resolve(probe) {
                    assert!(
                        known.contains(&res.address),
                        "resolve({probe:?}) returned {:?}, which is not an address in the catalogue",
                        res.address
                    );
                }
            }
        }
    }
}
