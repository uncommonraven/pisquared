//! The link check: every image address in the catalogue, requested with a HEAD request.
//!
//! Each distinct address is requested once, and every address that does not resolve is reported by name. The failure
//! it catches is the publisher renaming or removing an image, which would otherwise reach an operator as a 404 partway
//! through a 200 MiB download. It does not catch the publisher adding a board, because nothing tells us to look.
//!
//! The check reaches the publisher's servers, so nothing runs it on a push. The `catalogue-addresses` CI job runs it
//! on demand, through the ignored test at the end of this module.

use std::fmt;
use std::time::Duration;

use crate::backend::catalogue::Device;

/// Bounds a single HEAD request. Generous for a CDN, short enough that a failed link is reported rather than
/// appearing to hang.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The client the check runs with against the publisher.
pub fn default_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build()
}

/// One address that did not resolve, and the devices that wanted it.
#[derive(Debug)]
pub struct Failure<'a> {
    /// The address that was requested and did not resolve.
    pub address: String,
    /// The catalogue entries that resolve to it. Several devices share one image, so a failure names every
    /// name an operator might recognise rather than only the first.
    pub devices: Vec<&'a Device>,
    /// What came back: the server's own status, or the error the request failed with. Kept verbatim, because
    /// a 404, a 403, and a timeout each point at a different fix.
    pub reason: String,
}

impl fmt::Display for Failure<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<String> = self
            .devices
            .iter()
            .map(|d| format!("{} ({})", d.name, d.id))
            .collect();
        write!(
            f,
            "{} does not resolve: {}. The catalogue sends {} to it.",
            self.address,
            self.reason,
            names.join(", ")
        )
    }
}

/// Renders failures for the check's output, or an empty string when there were none.
pub fn report(failures: &[Failure<'_>]) -> String {
    if failures.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "{} of the catalogue's image addresses did not resolve:\n",
        failures.len()
    );
    for failure in failures {
        out.push_str(&format!("  {failure}\n"));
    }
    out
}

/// Requests each distinct address under `base` once and returns every one that did not resolve, in the order
/// the addresses first appear. An empty result means every address resolved.
///
/// `base` is the publisher's images directory for the real check. Tests pass a local server instead, so the
/// request code that checks the publisher is the code the tests run.
pub async fn check<'a>(
    client: &reqwest::Client,
    base: &str,
    devices: &'a [Device],
) -> Vec<Failure<'a>> {
    let mut failures = Vec::new();
    for (address, group) in address_groups(base, devices) {
        let reason = match client.head(&address).send().await {
            Ok(response) if response.status() == reqwest::StatusCode::OK => continue,
            Ok(response) => response.status().to_string(),
            Err(err) => err.to_string(),
        };
        failures.push(Failure {
            address,
            devices: group,
            reason,
        });
    }
    failures
}

/// Collapses devices to one entry per distinct address, keeping the order the addresses first appear in.
///
/// Devices that share an image are checked once rather than once each. A failure still names every device that
/// wanted the address, so nothing is lost by asking once.
fn address_groups<'a>(base: &str, devices: &'a [Device]) -> Vec<(String, Vec<&'a Device>)> {
    let mut groups: Vec<(String, Vec<&'a Device>)> = Vec::new();
    for device in devices {
        let address = device.address_under(base);
        match groups.iter_mut().find(|(a, _)| *a == address) {
            Some((_, members)) => members.push(device),
            None => groups.push((address, vec![device])),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::catalogue::{ADDRESS_PREFIX, DEVICES};
    use crate::testing::test_server::{Reply, client, image_path, serve_logged};
    use std::collections::HashMap;

    /// A small catalogue for the check to run over. Two of its three entries share an address, so every test
    /// exercises the grouping rather than assuming one device means one request.
    fn fixture_devices() -> Vec<Device> {
        let device = |id, name, board| Device {
            id,
            name,
            board,
            arch: "ARMv8",
            codename: "Trixie",
            caveats: &[],
        };
        vec![
            device("alpha", "Alpha Board", "Alpha"),
            device("beta", "Beta Board", "Beta"),
            device("beta-late", "Beta Board, later revision", "Beta"),
        ]
    }

    /// The routes a server needs to answer every device in `devices` with 200, except the devices in `failing`,
    /// which answer with their given status.
    fn routes(devices: &[Device], failing: &[(&Device, u16)]) -> HashMap<String, Reply> {
        let mut routes: HashMap<String, Reply> = devices
            .iter()
            .map(|d| (image_path(&d.image_name()), Reply::Status(200)))
            .collect();
        for (device, status) in failing {
            routes.insert(image_path(&device.image_name()), Reply::Status(*status));
        }
        routes
    }

    /// The passing half of the check: a run in which every address resolves passes. A check that only ever reports
    /// failures is indistinguishable from one wired up wrongly.
    #[tokio::test]
    async fn passes_when_every_address_resolves() {
        let devices = fixture_devices();
        let (base, requests) = serve_logged(routes(&devices, &[]));

        let failures = check(&client(), &base, &devices).await;
        assert!(failures.is_empty(), "{}", report(&failures));
        assert_eq!(report(&failures), "");

        // Three devices, two distinct addresses, two requests, each a HEAD.
        let requests = requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 2, "requests: {requests:?}");
        for request in &requests {
            assert!(
                request.starts_with("HEAD "),
                "the check issued {request:?}, but a HEAD request is what checks an address without fetching it"
            );
        }
    }

    /// The failure names the entry and the address that stopped resolving. A log saying only "link check failed"
    /// sends a reader to grep; one naming the address and the board does not.
    #[tokio::test]
    async fn names_the_entry_and_the_address_that_stopped_resolving() {
        let devices = fixture_devices();
        let broken = &devices[1];
        let (base, _) = serve_logged(routes(&devices, &[(broken, 404)]));

        let failures = check(&client(), &base, &devices).await;
        assert_eq!(failures.len(), 1, "{}", report(&failures));

        let failure = &failures[0];
        assert_eq!(failure.address, broken.address_under(&base));
        assert!(
            failure.reason.contains("404"),
            "reason {:?} keeps the server's status",
            failure.reason
        );

        // Both devices on the broken address are named, since either reader may own one of them.
        let named: Vec<&str> = failure.devices.iter().map(|d| d.name).collect();
        for want in ["Beta Board", "Beta Board, later revision"] {
            assert!(
                named.contains(&want),
                "the failure does not name {want:?}; it names {named:?}"
            );
        }

        let message = failure.to_string();
        for want in [
            failure.address.as_str(),
            "404",
            "Beta Board",
            "beta",
            "beta-late",
        ] {
            assert!(
                message.contains(want),
                "the failure message does not contain {want:?}:\n{message}"
            );
        }

        // The addresses that resolved are not reported.
        assert!(
            failures
                .iter()
                .all(|f| f.address != devices[0].address_under(&base))
        );
    }

    /// The check does not stop at the first breakage. A build that reports one and exits sends a reviewer to fix
    /// one problem and push again to learn about the rest.
    #[tokio::test]
    async fn reports_every_address_that_does_not_resolve() {
        let devices = fixture_devices();
        let (base, _) = serve_logged(routes(&devices, &[(&devices[0], 404), (&devices[1], 500)]));

        let failures = check(&client(), &base, &devices).await;
        assert_eq!(failures.len(), 2, "{}", report(&failures));

        let text = report(&failures);
        for want in [
            "404",
            "500",
            failures[0].address.as_str(),
            failures[1].address.as_str(),
        ] {
            assert!(
                text.contains(want),
                "the report does not mention {want:?}:\n{text}"
            );
        }
    }

    /// A network that is down, a refusing proxy, or a typo in the host leaves every address unreachable.
    /// Reporting that as a pass would look exactly like a working catalogue, which is the worst outcome available.
    #[tokio::test]
    async fn reports_an_unreachable_server_rather_than_passing() {
        // Port 1 on loopback has nothing listening, so the connection is refused at once.
        let devices = fixture_devices();
        let failures = check(&client(), "http://127.0.0.1:1/images/", &devices).await;

        assert_eq!(
            failures.len(),
            2,
            "nothing answered, so every distinct address must fail: {}",
            report(&failures)
        );
        for failure in &failures {
            assert!(
                !failure.reason.is_empty(),
                "the failure for {} carries no reason",
                failure.address
            );
        }
    }

    /// Reaches the publisher's servers, so it is ignored by default and run by the `catalogue-addresses` CI job.
    #[tokio::test]
    #[ignore = "reaches the publisher's servers; run with `cargo test -- --ignored`"]
    async fn every_committed_address_resolves() {
        let client = default_client().expect("client");
        let failures = check(&client, ADDRESS_PREFIX, DEVICES).await;
        assert!(failures.is_empty(), "{}", report(&failures));
    }
}
