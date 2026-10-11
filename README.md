# pisquared

Builds a self-hosted cloud agent host on a Raspberry Pi. The end state is the [pi](https://github.com/earendil-works/pi)
agent harness running in a kernel-enforced sandbox, reachable over an overlay network, installed from a repository
you can read. pisquared is one interface that takes you from choosing a Raspberry Pi to a flashed card that boots
your agent host, built one version at a time along the [roadmap](#roadmap), each ending on a result someone else
can check rather than take on trust. **The project is at v0.1.** Nothing below describes a finished program. v0.1
is implemented in Rust.

## What the tool does

When complete, one interface walks you through:

- [x] choosing the Raspberry Pi device you own,
- [ ] choosing PREFERENCES: orchestrator (herdr), harness (pi, opencode, or Claude Code), and sandbox (nono),
- [ ] entering CREDENTIALS: your Wi-Fi network name and passkey, and a Tailscale auth key,
- [ ] choosing the drive to flash,
- [ ] confirming the choices,
- [x] downloading the DietPi image for the device and checking it against its published checksum and signature,
- [x] decompressing it,
- [ ] writing your choices into the image's boot partition,
- [ ] flashing the image, with an elevated-privilege prompt for that step only,
- [ ] deleting every file it created along the way.

## Operating system

pisquared uses [DietPi](https://dietpi.com) as its operating system layer, chosen for the optimisations and
efficiency it shows on Raspberry Pi devices. See DietPi's [benchmark page](https://dietpi.com/stats.html).

pisquared is completely independent of DietPi and is not affiliated with it.

## What v0.1 produces

**A verified compressed image, decompressed to a directly flashable file, and nothing else.**

You name the device you own. The tool resolves it to the DietPi image that boots it, fetches the compressed image,
checks it against the checksum and detached signature DietPi publishes for it, and decompresses it. The image is
unmodified upstream bytes. Nothing this project produces is in it, and the checksum and signature cover the
compressed download.

## Working features

| Feature | Description |
|---|---|
| Device resolution | Maps a device family to the image that boots it. Four images cover fourteen devices: the Pi 4, Pi 400, and Compute Module 4 share one image, and the Pi 2 needs two for its two board revisions. A device not in the table resolves to no image, so a wrong image is caught before a card is erased. |
| Address check | An on-demand CI job confirms every image address in the table still resolves, so an upstream rename is caught before an operator meets it as a 404 mid-download. |
| Download and verification | Fetches the compressed image and checks it against its published checksum and detached signature. |
| Decompression | Produces a directly flashable file. |
| Interface | A [ratatui](https://ratatui.rs) interface over a step log. Deciding and work live in packages that never import the interface, so the whole tool runs and is tested with no terminal attached. |

Not yet working: writing choices into the image (v0.2), flashing, privilege elevation, preference and credential
screens, and saving a record file. The step log is shown in the interface only.

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

Running `target/release/pisquared` opens the interface. It takes no arguments yet.

CI in [`.github/workflows/rust.yml`](.github/workflows/rust.yml) runs format, lint, and test checks on Linux,
macOS, and Windows on every push to `main` and on every pull request. Platform-specific failures, such as path
separators or files that cannot be unlinked while open, are caught there rather than by an operator.

## Roadmap

### Target

**v1.0 is full functionality:** a setup tool that configures and flashes a Raspberry Pi, agent sandboxing, a
multiplexer, secure connectivity, and MCP server installation, with a documentation site.

### Versions

| Version | Result | Check |
|---|---|---|
| **v0.1** | A verified compressed image, decompressed to a flashable file | the compressed image matches its published checksum, and its signature verifies |
| **v0.2** | An image carrying the operator's choices | differs from upstream in exactly the documented fields; no flash, no elevation |
| **v0.3** | A device reachable without a console | flash, power on with nothing attached, key login, password refused |
| **v0.4** | One agent session completes | a session runs end to end |
| **v0.5** | The agent is contained | write inside the workspace succeeds, read outside returns `EPERM` |
| **v0.6** | Sessions survive a disconnect | detach, reconnect from another device, same session and scrollback |
| **v0.7** | Overlay is the only ingress | reachable over the overlay, LAN path closed |
| **v0.8** | A host can be rebuilt from the repository | fresh clone and wiped card produce an identical host |
| **v1.0** | Each MCP server is confined | every server under a policy narrower than the agent's |

## Licence

Not yet chosen.
