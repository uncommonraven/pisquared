//! The interface as a whole: an opening splash, choosing a device, choosing its preferences, entering its network connection,
//! confirming the choices, then obtaining its image while showing progress, then the outcome.
//!
//! The splash opens the interface with a choice of Begin or Quit. Before a run starts the device selection view has the
//! screen once the splash is passed, then the preferences view once a device is chosen, then the network view once the
//! preferences are complete, then the confirmation view once the network is valid. Only Enter on the confirmation view
//! starts the run, so nothing is downloaded until the operator has seen their choices. While a run is in progress the screen shows
//! what it is doing and Esc or Ctrl-C cancels it. Once the run has ended the screen shows its outcome and its log, and
//! Esc or Ctrl-C quits.

use std::cmp::Ordering;
use std::path::PathBuf;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Gauge, Paragraph, Wrap};

use crate::backend::catalogue::Device;
use crate::backend::fetch::Verification;
use crate::backend::network::Connection;
use crate::backend::preferences::Preferences;
use crate::backend::run::{Activity, Job, Obtained, RunError};
use crate::frontend::{hostname, network, preferences, select, splash};

pub struct Model {
    splash: Option<splash::Model>,
    select: select::Model,
    choosing: Option<Choosing>,
    credentialing: Option<Credentialing>,
    hostnaming: Option<Hostnaming>,
    confirming: Option<Confirming>,
    output_dir: PathBuf,
    run: Option<Run>,
}

/// A chosen device whose preferences are being picked before its image is obtained.
struct Choosing {
    device: &'static Device,
    menus: preferences::Model,
}

/// A chosen device whose complete preferences are followed by its network connection, entered before anything is downloaded.
struct Credentialing {
    device: &'static Device,
    menus: preferences::Model,
    preferences: Preferences,
    network: network::Model,
    hostname: hostname::Model,
}

/// A chosen device whose complete preferences and valid network are followed by its hostname.
struct Hostnaming {
    device: &'static Device,
    menus: preferences::Model,
    preferences: Preferences,
    network: network::Model,
    hostname: hostname::Model,
}

/// A chosen device whose complete preferences, valid network, and valid hostname are shown for the operator to confirm
/// before anything is downloaded.
struct Confirming {
    device: &'static Device,
    menus: preferences::Model,
    preferences: Preferences,
    network: network::Model,
    hostname: hostname::Model,
    connection: Connection,
    name: String,
}

enum Run {
    Active { job: Job, preferences: Preferences },
    Ended { outcome: String, log: String },
}

impl Model {
    pub fn new(output_dir: PathBuf) -> Self {
        Self {
            splash: Some(splash::Model::default()),
            select: select::Model::default(),
            choosing: None,
            credentialing: None,
            hostnaming: None,
            confirming: None,
            output_dir,
            run: None,
        }
    }
}

/// Applies a terminal event and reports whether the interface should quit.
pub fn update(model: &mut Model, event: &Event) -> bool {
    let Event::Key(key) = event else {
        return false;
    };
    if key.kind != KeyEventKind::Press {
        return false;
    }
    let interrupt = key.code == KeyCode::Esc
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL));

    match &model.run {
        Some(Run::Active { job, .. }) => {
            if interrupt {
                job.cancel();
            }
            false
        }
        Some(Run::Ended { .. }) => interrupt,
        None => {
            if let Some(splash) = model.splash.as_mut() {
                return match splash::update(splash, event) {
                    Some(splash::Choice::Begin) => {
                        model.splash = None;
                        false
                    }
                    Some(splash::Choice::Quit) => true,
                    None => false,
                };
            }
            if let Some(confirming) = model.confirming.take() {
                return confirm(model, confirming, key);
            }
            if let Some(hostnaming) = model.hostnaming.take() {
                return name_host(model, hostnaming, key);
            }
            if let Some(credentialing) = model.credentialing.take() {
                return credential(model, credentialing, key);
            }
            if interrupt {
                return true;
            }
            if let Some(mut choosing) = model.choosing.take() {
                match key.code {
                    KeyCode::Enter => match choosing.menus.enter() {
                        Some(preferences) => {
                            model.credentialing = Some(Credentialing {
                                device: choosing.device,
                                menus: choosing.menus,
                                preferences,
                                network: network::Model::default(),
                                hostname: hostname::Model::default(),
                            });
                        }
                        None => model.choosing = Some(choosing),
                    },
                    KeyCode::Left => {
                        if choosing.menus.back() {
                            model.choosing = Some(choosing);
                        }
                    }
                    _ => {
                        preferences::update(&mut choosing.menus, event);
                        model.choosing = Some(choosing);
                    }
                }
            } else if key.code == KeyCode::Enter
                && let Some(device) = model.select.chosen()
            {
                model.choosing = Some(Choosing {
                    device,
                    menus: preferences::Model::default(),
                });
            } else {
                select::update(&mut model.select, event);
            }
            false
        }
    }
}

/// Applies a key on the Network page: Enter continues once the connection is valid, Left or Esc goes back to the
/// preferences with the choices kept, and Ctrl-C quits. Any other key edits the page.
fn credential(model: &mut Model, mut credentialing: Credentialing, key: &KeyEvent) -> bool {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return true;
    }
    match key.code {
        KeyCode::Enter => match credentialing.network.connection() {
            Some(_) => {
                credentialing.network.leave();
                model.hostnaming = Some(Hostnaming {
                    device: credentialing.device,
                    menus: credentialing.menus,
                    preferences: credentialing.preferences,
                    network: credentialing.network,
                    hostname: credentialing.hostname,
                });
            }
            None => model.credentialing = Some(credentialing),
        },
        KeyCode::Esc | KeyCode::Left => {
            credentialing.network.leave();
            model.choosing = Some(Choosing {
                device: credentialing.device,
                menus: credentialing.menus,
            });
        }
        _ => {
            network::update(&mut credentialing.network, key);
            model.credentialing = Some(credentialing);
        }
    }
    false
}

/// Applies a key on the Hostname page: Enter continues once the hostname is valid, Left or Esc goes back to the Network
/// page with its values kept, and Ctrl-C quits. Any other key edits the page.
fn name_host(model: &mut Model, mut hostnaming: Hostnaming, key: &KeyEvent) -> bool {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return true;
    }
    match key.code {
        KeyCode::Enter => match (hostnaming.network.connection(), hostnaming.hostname.name()) {
            (Some(connection), Some(name)) => {
                model.confirming = Some(Confirming {
                    device: hostnaming.device,
                    menus: hostnaming.menus,
                    preferences: hostnaming.preferences,
                    network: hostnaming.network,
                    hostname: hostnaming.hostname,
                    connection,
                    name,
                });
            }
            _ => model.hostnaming = Some(hostnaming),
        },
        KeyCode::Esc | KeyCode::Left => {
            model.credentialing = Some(Credentialing {
                device: hostnaming.device,
                menus: hostnaming.menus,
                preferences: hostnaming.preferences,
                network: hostnaming.network,
                hostname: hostnaming.hostname,
            });
        }
        _ => {
            hostname::update(&mut hostnaming.hostname, key);
            model.hostnaming = Some(hostnaming);
        }
    }
    false
}

/// Applies a key on the confirmation view: Enter starts the run, Esc or Left returns to the Hostname page with its values
/// kept, and Ctrl-C quits. Any other key leaves the confirmation as it is.
fn confirm(model: &mut Model, confirming: Confirming, key: &KeyEvent) -> bool {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return true;
    }
    match key.code {
        KeyCode::Enter => {
            model.run = Some(Run::Active {
                job: Job::start(confirming.device.id, model.output_dir.clone()),
                preferences: confirming.preferences,
            });
        }
        KeyCode::Esc | KeyCode::Left => {
            model.hostnaming = Some(Hostnaming {
                device: confirming.device,
                menus: confirming.menus,
                preferences: confirming.preferences,
                network: confirming.network,
                hostname: confirming.hostname,
            });
        }
        _ => model.confirming = Some(confirming),
    }
    false
}

/// Takes in what an active run has reported, and moves to its outcome once the run has ended.
pub fn tick(model: &mut Model) {
    let Some(Run::Active { job, .. }) = &mut model.run else {
        return;
    };
    if let Some((result, log)) = job.poll() {
        model.run = Some(Run::Ended {
            outcome: outcome(&result),
            log: log.to_string(),
        });
    }
}

/// The sections in order, each with its name and how many pages it has.
const SECTIONS: [(&str, usize); 5] = [
    ("Device", 1),
    ("Preferences", 3),
    ("Credentials", 2),
    ("Confirm", 1),
    ("Download", 1),
];

/// The section the operator is in, and the page within it, both counted from the first.
fn position(model: &Model) -> (usize, usize) {
    match (
        &model.run,
        &model.confirming,
        &model.hostnaming,
        &model.credentialing,
        &model.choosing,
    ) {
        (Some(_), _, _, _, _) => (4, 0),
        (None, Some(_), _, _, _) => (3, 0),
        (None, None, Some(_), _, _) => (2, 1),
        (None, None, None, Some(_), _) => (2, 0),
        (None, None, None, None, Some(choosing)) => (1, choosing.menus.page()),
        (None, None, None, None, None) => (0, 0),
    }
}

/// Every section in order, with those before the current one marked as done. A current section with more than one
/// page shows its position beside its name, such as "Preferences 2/3".
fn progress(model: &Model) -> Line<'static> {
    let (current, page) = position(model);
    let mut spans = Vec::new();
    for (index, (name, pages)) in SECTIONS.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" › ", Style::new().dim()));
        }
        let (label, style) = match index.cmp(&current) {
            Ordering::Less => (format!("✓ {name}"), Style::new().fg(Color::Green)),
            Ordering::Equal if *pages > 1 => (
                format!("{name} {}/{pages}", page + 1),
                Style::new().bold().reversed(),
            ),
            Ordering::Equal => (name.to_string(), Style::new().bold().reversed()),
            Ordering::Greater => (name.to_string(), Style::new().dim()),
        };
        spans.push(Span::styled(label, style));
    }
    Line::from(spans)
}

pub fn view(model: &Model, frame: &mut Frame) {
    if let Some(splash) = &model.splash {
        splash::view(splash, frame, frame.area());
        return;
    }

    let [progress_area, body_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(frame.area());
    frame.render_widget(Paragraph::new(progress(model)), progress_area);

    match &model.run {
        None => match (
            &model.confirming,
            &model.hostnaming,
            &model.credentialing,
            &model.choosing,
        ) {
            (Some(confirming), _, _, _) => view_confirming(confirming, frame, body_area),
            (None, Some(hostnaming), _, _) => {
                hostname::view(&hostnaming.hostname, frame, body_area)
            }
            (None, None, Some(credentialing), _) => {
                network::view(&credentialing.network, frame, body_area)
            }
            (None, None, None, Some(choosing)) => {
                preferences::view(&choosing.menus, frame, body_area)
            }
            (None, None, None, None) => select::view(&model.select, frame, body_area),
        },
        Some(Run::Active { job, preferences }) => {
            view_active(job.current(), preferences, frame, body_area)
        }
        Some(Run::Ended { outcome, log }) => view_ended(outcome, log, frame, body_area),
    }
}

fn outcome(result: &Result<Obtained, RunError>) -> String {
    match result {
        Ok(obtained) => format!(
            "Obtained {}. {} The decompressed file is not verified, since the publisher publishes no value for it.\nPress Esc to quit.",
            obtained.flashable.display(),
            checksum_outcome(&obtained.verification)
        ),
        Err(err) => format!("{err}\nPress Esc to quit."),
    }
}

fn checksum_outcome(verification: &Verification) -> String {
    match verification {
        Verification::Verified { .. } => {
            "The download's checksum matches the one the publisher published, and its signature was verified."
                .to_owned()
        }
        Verification::Unverified { reason, .. } => format!(
            "The download's signature was verified, but its checksum was not: {reason}."
        ),
    }
}

fn view_active(activity: Activity, preferences: &Preferences, frame: &mut Frame, area: Rect) {
    let [status_area, progress_area, preferences_area, hint_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    frame.render_widget(Paragraph::new(describe(activity)), status_area);
    frame.render_widget(
        Gauge::default().ratio(ratio(activity)).label(""),
        progress_area,
    );
    frame.render_widget(Paragraph::new(preferences.to_string()), preferences_area);
    frame.render_widget(
        Paragraph::new(
            "Press Esc or Ctrl-C to cancel. A cancelled run leaves nothing that looks finished.",
        ),
        hint_area,
    );
}

fn view_confirming(confirming: &Confirming, frame: &mut Frame, area: Rect) {
    let device = confirming.device;
    let mut lines = vec![
        "Confirm your choices. Nothing has been downloaded yet.".to_owned(),
        format!("Device: {}", device.name),
        format!("Image: {}", device.image_name()),
        format!("Address: {}", device.address()),
    ];
    lines.extend(
        device
            .caveats
            .iter()
            .map(|caveat| format!("Caveat: {caveat}")),
    );
    lines.push(format!("Preferences: {}", confirming.preferences));
    lines.push(format!("Network: {}", confirming.connection));
    lines.push(format!("Hostname: {}", confirming.name));
    lines.push(
        "Press Enter to download and verify the image. Esc or Left goes back to change your choices. Ctrl-C quits.".to_owned(),
    );

    frame.render_widget(
        Paragraph::new(lines.join("\n")).wrap(Wrap { trim: true }),
        area,
    );
}

fn view_ended(outcome: &str, log: &str, frame: &mut Frame, area: Rect) {
    let [outcome_area, log_area] =
        Layout::vertical([Constraint::Length(4), Constraint::Fill(1)]).areas(area);

    frame.render_widget(
        Paragraph::new(outcome).wrap(Wrap { trim: true }),
        outcome_area,
    );
    frame.render_widget(
        Paragraph::new(log).block(Block::bordered().title("Log")),
        log_area,
    );
}

fn describe(activity: Activity) -> String {
    match activity {
        Activity::Resolving => "Resolving the device to its image".to_owned(),
        Activity::Downloading {
            written,
            total: Some(total),
        } => format!(
            "Downloading: {} of {} ({:.0}%)",
            mebibytes(written),
            mebibytes(total),
            ratio(activity) * 100.0
        ),
        Activity::Downloading {
            written,
            total: None,
        } => format!("Downloading: {}", mebibytes(written)),
        Activity::Verifying => "Verifying the checksum and the signature".to_owned(),
        Activity::Decompressing => "Decompressing the image".to_owned(),
    }
}

/// How far through the run's current step it is, from nothing to all of it.
fn ratio(activity: Activity) -> f64 {
    match activity {
        Activity::Resolving => 0.0,
        Activity::Downloading {
            written,
            total: Some(total),
        } if total > 0 => (written as f64 / total as f64).min(1.0),
        Activity::Downloading { .. } => 0.0,
        Activity::Verifying | Activity::Decompressing => 1.0,
    }
}

fn mebibytes(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::preferences::Harness;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    /// A model with the splash passed by choosing Begin, so the tests start on the device screen.
    fn started() -> Model {
        let mut model = Model::new(PathBuf::from("."));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));
        model
    }

    /// Picks the first device and every first option of the preferences, then continues past the last screen.
    fn complete_preferences(model: &mut Model) {
        update(model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(model, &key(KeyCode::Enter, KeyModifiers::NONE));
        update(model, &key(KeyCode::Enter, KeyModifiers::NONE));
        update(model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(model, &key(KeyCode::Enter, KeyModifiers::NONE));
        update(model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(model, &key(KeyCode::Enter, KeyModifiers::NONE));
        update(model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(model, &key(KeyCode::Enter, KeyModifiers::NONE));
    }

    fn type_text(model: &mut Model, text: &str) {
        for c in text.chars() {
            update(model, &key(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    /// Fills in a valid Wi-Fi network on the Network page and continues from it.
    fn complete_network(model: &mut Model) {
        update(model, &key(KeyCode::Down, KeyModifiers::NONE));
        type_text(model, "Home");
        update(model, &key(KeyCode::Down, KeyModifiers::NONE));
        type_text(model, "correct horse");
        update(model, &key(KeyCode::Down, KeyModifiers::NONE));
        type_text(model, "GB");
        update(model, &key(KeyCode::Enter, KeyModifiers::NONE));
    }

    /// Accepts the default hostname on the Hostname page and continues from it.
    fn complete_hostname(model: &mut Model) {
        update(model, &key(KeyCode::Enter, KeyModifiers::NONE));
    }

    #[test]
    fn the_splash_comes_first_and_quit_ends_the_interface() {
        let mut model = Model::new(PathBuf::from("."));

        assert!(model.splash.is_some(), "the splash opens the interface");
        assert!(!update(
            &mut model,
            &key(KeyCode::Char(' '), KeyModifiers::NONE)
        ));
        assert!(model.splash.is_some(), "other keys do not pass the splash");
        assert!(model.select.chosen().is_none());

        update(&mut model, &key(KeyCode::Down, KeyModifiers::NONE));
        assert!(update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE)));
    }

    #[test]
    fn begin_passes_the_splash_to_the_device_screen() {
        let mut model = Model::new(PathBuf::from("."));

        assert!(!update(
            &mut model,
            &key(KeyCode::Enter, KeyModifiers::NONE)
        ));

        assert!(model.splash.is_none());
        assert!(model.run.is_none());
    }

    #[test]
    fn the_progress_line_shows_bytes_and_percentage_of_the_declared_total() {
        let mib = 1024 * 1024;

        assert_eq!(
            describe(Activity::Downloading {
                written: mib,
                total: Some(4 * mib),
            }),
            "Downloading: 1.0 MiB of 4.0 MiB (25%)"
        );
        assert_eq!(
            describe(Activity::Downloading {
                written: mib,
                total: None,
            }),
            "Downloading: 1.0 MiB"
        );
    }

    #[test]
    fn space_picks_a_device_and_enter_continues_without_starting_it() {
        let mut model = started();

        assert!(!update(
            &mut model,
            &key(KeyCode::Char(' '), KeyModifiers::NONE)
        ));
        assert!(model.select.chosen().is_some());
        assert!(model.choosing.is_none(), "picking a device moves nothing");

        assert!(!update(
            &mut model,
            &key(KeyCode::Enter, KeyModifiers::NONE)
        ));
        assert!(
            model.choosing.is_some(),
            "Enter continues to the preferences"
        );
        assert!(model.run.is_none(), "choosing a device starts nothing");
    }

    #[test]
    fn enter_without_a_pick_does_nothing_and_escape_quits() {
        let mut model = started();

        assert!(!update(
            &mut model,
            &key(KeyCode::Enter, KeyModifiers::NONE)
        ));
        assert!(model.choosing.is_none());

        assert!(update(&mut model, &key(KeyCode::Esc, KeyModifiers::NONE)));
    }

    #[test]
    fn an_outcome_states_the_checksum_as_it_was_found() {
        let obtained = |verification| {
            outcome(&Ok(Obtained {
                image: PathBuf::from("image.img.xz"),
                flashable: PathBuf::from("image.img"),
                verification,
            }))
        };

        let unverified = obtained(Verification::Unverified {
            source: "https://example.com/image.img.xz.sha256".to_owned(),
            reason: "the server answered 404 Not Found".to_owned(),
        });
        assert!(unverified.contains("404 Not Found"), "{unverified}");
        assert!(!unverified.contains("matches"), "{unverified}");

        let verified = obtained(Verification::Verified {
            source: "https://example.com/image.img.xz.sha256".to_owned(),
            digest: "00".to_owned(),
        });
        assert!(verified.contains("matches"), "{verified}");
    }

    #[test]
    fn a_chosen_device_opens_its_preferences_and_an_incomplete_choice_starts_nothing() {
        let mut model = started();
        update(&mut model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));

        assert!(model.choosing.is_some(), "the preferences screen is shown");
        assert!(model.run.is_none());

        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));

        assert!(
            model.choosing.is_some(),
            "the preferences are not yet complete"
        );
        assert!(model.run.is_none());
    }

    #[test]
    fn complete_preferences_lead_through_the_network_and_hostname_pages_to_the_confirmation() {
        let mut model = started();
        complete_preferences(&mut model);

        assert!(model.credentialing.is_some(), "the network page is shown");
        assert!(model.confirming.is_none(), "the network is not yet entered");
        assert!(model.run.is_none());

        complete_network(&mut model);

        assert!(model.credentialing.is_none());
        assert!(model.hostnaming.is_some(), "the hostname page is shown");
        assert!(
            model.confirming.is_none(),
            "the hostname is not yet entered"
        );

        complete_hostname(&mut model);

        assert!(model.hostnaming.is_none());
        assert!(model.confirming.is_some(), "the confirmation is shown");
        assert!(model.choosing.is_none());
        assert!(model.run.is_none(), "confirming starts nothing");
    }

    #[test]
    fn an_invalid_hostname_does_not_continue_to_the_confirmation() {
        let mut model = started();
        complete_preferences(&mut model);
        complete_network(&mut model);
        for _ in 0..9 {
            update(&mut model, &key(KeyCode::Backspace, KeyModifiers::NONE));
        }

        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));

        assert!(model.hostnaming.is_some(), "the hostname is not yet valid");
        assert!(model.confirming.is_none());
    }

    #[test]
    fn ethernet_skips_the_wifi_fields_and_goes_on_to_the_hostname_page() {
        let mut model = started();
        complete_preferences(&mut model);
        update(&mut model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));

        assert!(model.credentialing.is_none());
        assert!(model.hostnaming.is_some(), "the hostname page is shown");

        complete_hostname(&mut model);

        assert!(model.hostnaming.is_none());
        assert!(matches!(
            model
                .confirming
                .as_ref()
                .map(|confirming| &confirming.connection),
            Some(Connection::Ethernet)
        ));
        assert!(model.run.is_none(), "confirming starts nothing");
    }

    #[test]
    fn an_invalid_network_page_does_not_continue() {
        let mut model = started();
        complete_preferences(&mut model);

        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));

        assert!(
            model.credentialing.is_some(),
            "the network is not yet valid"
        );
        assert!(model.confirming.is_none());
    }

    #[test]
    fn escape_on_the_network_page_returns_to_the_preferences_and_keeps_them() {
        let mut model = started();
        complete_preferences(&mut model);
        complete_network(&mut model);
        update(&mut model, &key(KeyCode::Esc, KeyModifiers::NONE));

        assert!(!update(&mut model, &key(KeyCode::Esc, KeyModifiers::NONE)));

        assert!(model.credentialing.is_none());
        let choosing = model.choosing.as_ref().expect("back on the preferences");
        assert_eq!(
            choosing.menus.confirmed().map(|p| p.harnesses),
            Some(vec![Harness::Pi])
        );
        assert!(model.run.is_none());
    }

    #[test]
    fn escape_on_the_confirmation_returns_to_the_hostname_page_and_keeps_the_network() {
        let mut model = started();
        complete_preferences(&mut model);
        complete_network(&mut model);
        complete_hostname(&mut model);

        assert!(!update(&mut model, &key(KeyCode::Esc, KeyModifiers::NONE)));

        assert!(model.confirming.is_none());
        let hostnaming = model
            .hostnaming
            .as_ref()
            .expect("back on the hostname page");
        assert!(matches!(
            hostnaming.network.connection(),
            Some(Connection::Wifi(wifi)) if wifi.ssid == "Home"
        ));
        assert!(model.run.is_none());
    }

    #[test]
    fn the_confirmation_shows_the_choices_before_the_download() {
        let mut model = started();
        complete_preferences(&mut model);
        complete_network(&mut model);
        complete_hostname(&mut model);

        let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
        terminal.draw(|frame| view(&model, frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let rendered: String = (0..20)
            .map(|y| {
                (0..120)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        let rendered = rendered.split_whitespace().collect::<Vec<_>>().join(" ");

        assert!(
            rendered.contains("Nothing has been downloaded yet."),
            "{rendered}"
        );
        assert!(
            rendered.contains(
                "Preferences: Orchestrator: herdr. Harnesses: pi agent. Sandboxes: nono."
            ),
            "{rendered}"
        );
        assert!(
            rendered.contains(
                "Network: Wi-Fi SSID \"Home\", passphrase ********, country GB (United Kingdom"
            ),
            "{rendered}"
        );
        assert!(rendered.contains("Hostname: pisquared"), "{rendered}");
        assert!(!rendered.contains("correct horse"), "{rendered}");
    }

    #[test]
    fn left_goes_back_a_stage_through_each_page() {
        let mut model = started();
        update(&mut model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));

        assert!(!update(&mut model, &key(KeyCode::Left, KeyModifiers::NONE)));
        assert!(model.choosing.is_none(), "back on the device selection");
        assert!(
            model.select.chosen().is_some(),
            "the device is still chosen"
        );

        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(model.credentialing.is_some());

        update(&mut model, &key(KeyCode::Left, KeyModifiers::NONE));
        assert!(model.credentialing.is_none());
        assert!(model.choosing.is_some(), "back on the preferences");

        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(model.credentialing.is_some());
        complete_network(&mut model);
        assert!(model.hostnaming.is_some());
        complete_hostname(&mut model);
        assert!(model.confirming.is_some());

        update(&mut model, &key(KeyCode::Left, KeyModifiers::NONE));
        assert!(model.confirming.is_none());
        assert!(model.hostnaming.is_some(), "back on the hostname page");

        update(&mut model, &key(KeyCode::Left, KeyModifiers::NONE));
        assert!(model.hostnaming.is_none());
        assert!(model.credentialing.is_some(), "back on the network page");
        assert!(model.run.is_none());
    }

    #[test]
    fn the_progress_line_marks_the_pages_already_done() {
        let mut model = started();
        update(&mut model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));

        let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
        terminal.draw(|frame| view(&model, frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let progress: String = (0..120).map(|x| buffer[(x, 0)].symbol()).collect();

        assert!(
            progress.contains("✓ Device › Preferences 2/3 › Credentials › Confirm › Download"),
            "{progress}"
        );
    }

    #[test]
    fn the_network_page_sits_between_the_preferences_and_the_confirmation_in_the_progress_line() {
        let mut model = started();
        complete_preferences(&mut model);

        let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
        terminal.draw(|frame| view(&model, frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let progress: String = (0..120).map(|x| buffer[(x, 0)].symbol()).collect();

        assert!(
            progress.contains("✓ Device › ✓ Preferences › Credentials 1/2 › Confirm › Download"),
            "{progress}"
        );
    }

    #[test]
    fn the_hostname_page_is_the_second_credentials_page_in_the_progress_line() {
        let mut model = started();
        complete_preferences(&mut model);
        complete_network(&mut model);

        let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
        terminal.draw(|frame| view(&model, frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let progress: String = (0..120).map(|x| buffer[(x, 0)].symbol()).collect();

        assert!(
            progress.contains("✓ Device › ✓ Preferences › Credentials 2/2 › Confirm › Download"),
            "{progress}"
        );
    }

    #[test]
    fn control_c_quits_before_a_run() {
        let mut model = started();

        assert!(update(
            &mut model,
            &key(KeyCode::Char('c'), KeyModifiers::CONTROL)
        ));
    }
}
