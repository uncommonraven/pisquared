//! The build-time link check.
//!
//! Every image address committed in the catalogue is requested with a HEAD request, once per distinct
//! address, and every address that does not resolve is reported by name. The failure it catches is
//! upstream renaming or removing an image, which would otherwise reach an operator as a 404 partway
//! through a 200 MiB download. It does not catch upstream adding a board, because nothing tells us to look.

use std::fmt;
use std::time::Duration;

use crate::catalogue::Device;

/// Bounds a single HEAD request. Generous for a CDN, short enough that a failed link is reported rather than
/// appearing to hang.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The client `check` is run with in the build.
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

/// Renders failures for build output, or an empty string when there were none.
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
/// `base` is `ADDRESS_PREFIX` in the build. Tests pass a local server instead, so the request code that runs in
/// the build is the code the tests run.
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
        let address = format!("{base}{}", device.image_name());
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
    use crate::catalogue::{ADDRESS_PREFIX, DEVICES};
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;

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

    /// The path a device's address takes under the local server's base.
    fn path_of(d: &Device) -> String {
        format!("/downloads/images/{}", d.image_name())
    }

    /// A local HTTP server answering HEAD requests with the status configured for each path, and recording the
    /// requests it was sent so a test can assert on them as well as on the outcome.
    struct Server {
        base: String,
        requests: Arc<Mutex<Vec<String>>>,
    }

    impl Server {
        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    fn serve(status: HashMap<String, u16>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let port = listener.local_addr().expect("local address").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&requests);
        let status = Arc::new(status);

        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let status = Arc::clone(&status);
                let log = Arc::clone(&log);
                thread::spawn(move || answer(stream, &status, &log));
            }
        });

        Server {
            base: format!("http://127.0.0.1:{port}/downloads/images/"),
            requests,
        }
    }

    fn answer(mut stream: TcpStream, status: &HashMap<String, u16>, log: &Mutex<Vec<String>>) {
        let mut head = Vec::new();
        let mut buf = [0u8; 1024];
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => head.extend_from_slice(&buf[..n]),
            }
        }

        let text = String::from_utf8_lossy(&head);
        let line = text.lines().next().unwrap_or_default();
        let mut parts = line.split(' ');
        let method = parts.next().unwrap_or_default();
        let path = parts.next().unwrap_or_default();
        log.lock().unwrap().push(format!("{method} {path}"));

        let code = status.get(path).copied().unwrap_or(200);
        let reason = match code {
            200 => "OK",
            404 => "Not Found",
            500 => "Internal Server Error",
            _ => "Unknown",
        };
        let response =
            format!("HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let _ = stream.write_all(response.as_bytes());
    }

    /// A client that does not route loopback requests through a proxy from the environment, and that gives up
    /// quickly, since everything it talks to here is local.
    fn test_client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .expect("client")
    }

    /// The passing half of task 3.5: a run in which every address resolves passes. A check that only ever reports
    /// failures is indistinguishable from one wired up wrongly.
    #[tokio::test]
    async fn passes_when_every_address_resolves() {
        let server = serve(HashMap::new());
        let devices = fixture_devices();

        let failures = check(&test_client(), &server.base, &devices).await;
        assert!(failures.is_empty(), "{}", report(&failures));
        assert_eq!(report(&failures), "");

        // Three devices, two distinct addresses, two requests.
        let requests = server.requests();
        assert_eq!(requests.len(), 2, "requests: {requests:?}");
        for request in &requests {
            assert!(
                request.starts_with("HEAD "),
                "the check issued {request:?}; design.md specifies HEAD"
            );
        }
    }

    /// The failing half of task 3.5: the failure names the entry and the address that stopped resolving. A build
    /// log saying "link check failed" sends a reader to grep; one naming the address and the board does not.
    #[tokio::test]
    async fn names_the_entry_and_the_address_that_stopped_resolving() {
        let devices = fixture_devices();
        let broken = &devices[1];
        let server = serve(HashMap::from([(path_of(broken), 404)]));

        let failures = check(&test_client(), &server.base, &devices).await;
        assert_eq!(failures.len(), 1, "{}", report(&failures));

        let failure = &failures[0];
        assert_eq!(
            failure.address,
            format!("{}{}", server.base, broken.image_name())
        );
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
                .all(|f| f.address != format!("{}{}", server.base, devices[0].image_name()))
        );
    }

    /// The check does not stop at the first breakage. A build that reports one and exits sends a reviewer to fix
    /// one problem and push again to learn about the rest.
    #[tokio::test]
    async fn reports_every_address_that_does_not_resolve() {
        let devices = fixture_devices();
        let server = serve(HashMap::from([
            (path_of(&devices[0]), 404),
            (path_of(&devices[1]), 500),
        ]));

        let failures = check(&test_client(), &server.base, &devices).await;
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
        let failures = check(
            &test_client(),
            "http://127.0.0.1:1/downloads/images/",
            &devices,
        )
        .await;

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
