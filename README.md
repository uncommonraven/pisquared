# pisquared

Builds a self-hosted cloud agent host on a Raspberry Pi. The end state is the [pi](https://github.com/earendil-works/pi)
agent harness running in a kernel-enforced sandbox, reachable over an overlay network, installable by you
from a repository you can read.

Work proceeds one version at a time along the ladder in [`ROADMAP.md`](ROADMAP.md). Each version ends on a
single observable result that someone else could check rather than take on trust. **The project is at v0.1.**
Nothing below describes a finished program.

v0.1 is implemented in Rust. The earlier Go implementation is kept in git history only.

## What v0.1 produces

**A verified, flashable base image, and nothing else.**

You name the device you own. The tool resolves it to the image that boots it, fetches it, checks it against
the checksum and detached signature its publisher publishes for it, decompresses it to a directly flashable
file, and writes a plain-text record beside the result naming what it was verified against.

**The image is unmodified upstream bytes.** Nothing this project produces is in it. It is a stock DietPi
release, byte for byte what the publisher serves. That is deliberate: it is what lets v0.1's exit criterion —
the tool and the hand-run procedure both compared against the publisher's own published hash — run with no
hardware involved. v0.2 is the first version that emits anything of ours.

Also in this version:

- A committed table mapping device families to image addresses, with a build-time check that every address in
  it still resolves, so upstream renaming an image fails here rather than on your machine mid-download.

  Four images cover fourteen devices. The Pi 4, the Pi 400, and the Compute Module 4 are one image and are
  listed as one choice. The Pi 2 is two, because its two board revisions need different ones. A device the
  table does not carry resolves to no image at all — there is no default and no nearest match, because a
  wrong image is discovered after a card has been erased rather than before.
- A [ratatui](https://ratatui.rs) interface that is a view over a step log. All
  deciding and all work happens in packages that never import the interface, so the whole thing runs and is
  tested with no terminal attached.
- A non-interactive mode that runs the identical code path and differs only in where the log is rendered.

## Getting an image without this tool

**Not written yet.** This version is also meant to produce a page you can follow without running anything
from this repository — [`docs/obtaining-a-verified-image.md`](docs/obtaining-a-verified-image.md) — and at
the moment you are reading this it does not exist. It is the next piece of work in v0.1, and this line
becomes a working link when it does. **If you want an image today, nothing on this page is the fastest
route**; the tool does not do it yet either.

That page will be written as though the tool did not exist: `curl` the address, `sha256sum -c` it against
the publisher's published file, `xz -d`, then flash the result with the publisher's documented flashing
tool. It will be complete enough to follow alone, including for a device the catalogue does not list.

It will be written *before* the tool, on purpose, and the tool will be checked against it. The document is
what survives this tool being deleted, replaced, or never run, which in a project about rebuilding a host
from its documentation is the artifact that matters.

## What this version does not do

- It does not flash anything. It produces a file. Writing that file to a card is a separate documented step
  using the publisher's documented tool, because the publisher documents no command-line flasher and
  inventing one would put an undocumented mechanism between you and an irreversible write.
- It does not mount, erase, or write to a storage device, and it never asks for elevated privileges.
- It does not write any pisquared configuration into the image. That is v0.2.
- It does not need a Raspberry Pi, and neither does its exit check. Hardware begins at v0.3, where a flashed
  device has to boot.

## Building

You need:

- **A Rust toolchain.** [rustup](https://rustup.rs) installs the exact version pinned in
  [`rust-toolchain.toml`](rust-toolchain.toml) the first time you build. The minimum supported version is
  recorded in [`Cargo.toml`](Cargo.toml).
- **A C compiler.** The xz decoder builds liblzma from source. Linux and macOS need `gcc` or `clang`. Windows
  needs the MSVC build tools (the "Desktop development with C++" workload in Visual Studio Build Tools). No
  system liblzma package is needed.

```sh
cargo build --release   # writes target/release/pisquared
cargo test
```

The binary currently prints its version. The download, verification, and decompression steps are not built yet.

Rust format, lint, and test checks run on Linux, macOS, and Windows in
[`.github/workflows/rust.yml`](.github/workflows/rust.yml) on every push to `main` and on every pull request. A
platform-specific failure — a path separator, a rename across devices, a file that cannot be unlinked while
open — is caught there rather than by an operator.

## The ladder

```
  v0.1  DOWNLOAD     tool resolves device -> verified image           no hardware
  v0.2  CONFIGURE    tool injects operator config into the image       no hardware
  v0.3  FIRST        flashed device boots headless to a keyed shell   HARDWARE
  v0.4  AGENT        pi installed, one session runs                   HARDWARE
  v0.5  CONTAINED    nono policy applied before first agent run        HARDWARE
  v0.6  DURABLE      herdr sessions survive a disconnect               HARDWARE
  v0.7  OVERLAY      Tailscale; the LAN path closes                    HARDWARE
  v0.8  REPRODUCIBLE wiped card + fresh clone -> identical host        HARDWARE
  v1.0  TOOLING      MCP servers, each under a narrower policy         HARDWARE
```

## Licence

Not yet chosen.