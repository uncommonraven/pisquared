The audit is done. I read all 13 files under src/ and ran cargo clippy --all-targets, which is clean. I did not run cargo test or change any code.
Dead code and misleading structure
1. src/tui.rs is leftover scaffolding. main.rs never uses it; it is only compiled under #[cfg(test)]. Its q-to-quit behaviour contradicts the real interface, which quits on Esc. Its five tests cover nothing the binary runs. Delete the file and the mod tui; line.
2. main.rs xz_round_trip_uses_the_decoder tests a third-party crate. It exercises async_compression's futures API, not pisquared code. decompress.rs already covers the real decoder end to end. Delete it. If futures is then unused, drop that dependency too.
3. check.rs is not a build-time check. It is #[cfg(test)], there is no build.rs, and its only live test is #[ignore]d and run by the manual catalogue-addresses job. The module doc and the README's "build-time check" wording both overstate this. Either wire it into a real build step or reword both.
4. catalogue.rs:12 points to a docs page that does not exist. MANUAL_PROCEDURE is docs/obtaining-a-verified-image.md, and there is no docs/ directory. The NotListed message sends operators there. The test only checks that the constant appears in the message, so nothing catches this.
Redundant or low-value tests
- signature.rs the_signing_key_is_named_where_the_publisher_publishes_it asserts a constant equals a string literal. It is a change-detector. Delete it.
- catalogue.rs every_identifier_resolves_to_its_own_device_and_address is tautological. resolve builds the address from d.address(), so the test re-checks the implementation against itself. Fold it into every_entry_carries_its_address..., or delete it.
- catalogue.rs near_misses_of_a_real_identifier... is effectively vacuous. resolve only succeeds on exact identifiers, and none of the probes is another identifier, so the assertion cannot fail. an_unlisted_device_resolves_to_no_image_and_no_default already covers these shapes. Delete it.
- catalogue.rs catalogue_carries_every_device_design_names_exactly_once copies the catalogue into a second list and compares. It also cites design.md, which is not in the repository. Keep the count check if you want it; the name list adds little.
- catalogue.rs address-shape tests (the_address_shape_rejects_*) test a hand-written validator that exists only in tests. They guard the oracle, not the product. Keep one positive and one negative case, or delete the validator and use url::Url checks.
- signature.rs the_key_and_signature_are_obtained_in_memory overclaims. Its name says "in memory", but it never checks that nothing was written to disk. Add the directory-listing check from a_good_signature_reports..., or rename it.
- Weak assertions: fetch.rs an_unreachable_checksum_is_unverified... and signature.rs an_unreachable_key_is_unverified... assert only that the reason is non-empty. Strengthen them to check the reason, or merge them with the 404 variants.
- Overlap: fetch.rs a_completed_transfer_appears... and a_matching_checksum_is_reported_verified run the same 3 MB download. an_interrupted_transfer... is subsumed by a_retry_after_an_interruption.... Merge each pair.
- Brittle prose assertions: run.rs a_full_run_writes_every_step_to_the_log asserts on wording such as "at the time of verification". Those assertions break on any copy edit. Assert on structure, or on one key fact.
Flakiness risk
- fetch.rs a_cancel_before_the_first_chunk... and a_cancel_during_a_transfer... fire cancels on wall-clock sleeps.
- decompress.rs a_cancelled_decode_leaves_nothing decodes 128 MiB and cancels after 50 ms. A fast machine can finish first and fail the test.
Cancelling from the progress callback once bytes have been written would make these deterministic. The decompression test needs an equivalent trigger.
Duplication
- payload, sha256_hex, xz/compress, and route-building helpers are copied across fetch.rs, decompress.rs, run.rs, and signature.rs.
- check.rs contains a second local HTTP server (serve, answer, test_client) because test_server.rs only handles GET. Extend test_server to support HEAD and delete the copy.
- test_publisher.rs repeats the hex formatting from signature::hex.
Consolidating these into one test-support module would remove most of the duplication.
Production code
- app.rs:92-100 hard-codes "The download was verified". A run can succeed when the checksum is unverified, for example after a 404 on the .sha256 file, because only the signature gates success. The outcome text then overclaims. Derive the wording from the Verification result.
- Address has two sources of truth. catalogue::Resolution.address is built from device.address(), but run.rs:82 ignores it and rebuilds the address from sources.images. select.rs:111-112 calls resolve(device.id).expect(...) only to read the address and caveats, which it could read from the device.
- Inconsistent halt behaviour. A refused signature removes the image. An unverified signature halts and leaves the compressed image at the output path, and signature.rs's test asserts that. Decide which is intended and document it.
- run.rs run() is about 120 lines. It handles five steps inline and repeats the cancellation mapping. Splitting it into one function per step would make the flow easier to read.
- decompress.rs:143-149 reads ErrorKind::OutOfMemory as truncation. A real allocation failure would be misreported the same way. The test pins the truncated case, so the risk is small, but a comment with the library reference would help.
- Stale comments cite design.md (not in the repo), "Task 4.1"-style numbers, "the 0.4 spike", and "the port". Remove or reword them.
If you want, I can make these changes. The highest-value items are the dead tui.rs, the misleading "verified" message in app.rs, the missing docs page, and the timing-based cancel tests.