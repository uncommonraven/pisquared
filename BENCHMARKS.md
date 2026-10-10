# xz decoder benchmarks

These benchmarks chose the xz decoder for the Rust port of pisquared (task 0.2 of the
`v0-1-image-download` change). The decision is recorded in
`openspec/changes/v0-1-image-download/design.md`.

## Input

- File: `DietPi_RPi5-ARMv8-Trixie.img.xz` (kept in `rustexperiment/`, excluded from version control)
- Compressed: 204.5 MiB
- Decompressed: 1,129.0 MiB (1,183,842,304 bytes, confirmed with `xz -l`)
- Measurement: single-threaded, output discarded, 1 MiB read buffer
- Machine: x86_64, 16 cores

## Round 1: Go vs. Rust

Source: `rustexperiment/README.md`. These results were recorded in an earlier session; they
were not re-run for this document.

| Decoder | Implementation | Elapsed | Throughput |
|---|---|---|---|
| Rust `xz2` | binding to system liblzma | ~5.3 s | ~214 MiB/s |
| Go `jamespfennell/xz` | cgo, system liblzma | ~5.9 s | ~186 MiB/s |
| Go `jamespfennell/xz` | cgo, vendored C (default build) | ~10.2 s | ~111 MiB/s |
| Go `ulikunitz/xz` | pure Go (the Go implementation in use) | ~49.2 s | ~22.9 MiB/s |

Findings:

- A C liblzma decoder is about 8× faster than the pure-Go decoder used by the Go implementation.
- Rust `xz2` and Go `jamespfennell/xz` on system liblzma call the same C library. The gap between
  them is cgo and Go overhead.
- The build mode matters. The default `jamespfennell/xz` build compiles a vendored subset of
  the lzma2 C source and runs about 2× slower than linking the system liblzma.

## Round 2: Rust crates

Run for task 0.2 in this session. Each candidate was built in two modes where the crate allows
it: linked against the system liblzma (`dyn`) and vendored into the binary (`static`). Each
figure is the median of the runs listed.

| Candidate | Build mode | Runs | Median | Throughput | Peak RSS |
|---|---|---|---|---|---|
| `async-compression` 0.4, `xz` feature | vendored (static) | 6 | 5.19 s | ~218 MiB/s | ~70 MB |
| `async-compression` 0.4, `xz` feature | system-linked | 3 | 4.45 s | ~254 MiB/s | ~70 MB |
| `xz2` 0.1.7 | vendored (static) | 3 | 8.20 s | ~138 MiB/s | not measured |
| `xz2` 0.1.7 | system-linked | 3 | 4.57 s | ~247 MiB/s | ~69 MB |
| `lzma-rs` 0.3 (pure Rust) | n/a | 3 | 13.12 s | ~86 MiB/s | ~397 MB |

Individual runs (seconds):

- `async-compression`, vendored: 5.195, 5.187, 5.185, 5.182, 5.166, 5.203
- `async-compression`, system: 4.476, 4.453, 4.442
- `xz2`, vendored: 8.196, 8.157, 8.199
- `xz2`, system: 4.775, 4.581, 4.574
- `lzma-rs`: 13.037, 13.124, 13.137

Peak RSS was measured with `/usr/bin/time -v` on one run per candidate.

### Verification

- Every run decompressed the same 1,183,842,304 bytes.
- `ldd` shows the system-linked builds depend on `liblzma.so.5`. The vendored builds have no
  liblzma shared-library dependency.

### Decision criteria

From `design.md`: a pure-Rust decoder is accepted only if it reaches at least 75% of the
fastest liblzma result and its memory stays bounded by the buffer.

- `lzma-rs` reaches about 40% of the fastest vendored liblzma result, and its peak memory is
  about 397 MB. It is rejected on both conditions.
- The fastest vendored liblzma build is `async-compression` (5.19 s). It was chosen, since the
  shipped binary is vendored.

Caveat: about 70 MB of peak memory is the liblzma dictionary, not the image or the buffer. It
does not grow with the image size, but it is not literally "bounded by the buffer." Task 6.1
should measure memory against image size.

## Notes

- Round 2 figures differ from the round 1 `xz2` figure (247 MiB/s here, 214 MiB/s in round 1).
  The two rounds were run on the same machine, but at different times, and round 1 was
  not re-run.