//! The interface as a whole: choosing a device, choosing its preferences, confirming them, then obtaining its image
//! while showing progress, then the outcome.
//!
//! Before a run starts the device selection view has the screen, then the preferences view once a device is chosen,
//! then the confirmation view once the preferences are complete. Only Enter on the confirmation view starts the run,
//! so nothing is downloaded until the operator has seen their choices. While a run is in progress the screen shows
//! what it is doing and Esc or Ctrl-C cancels it. Once the run has ended the screen shows its outcome and its log, and
//! Esc or Ctrl-C quits.

use std::path::PathBuf;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::widgets::{Block, Gauge, Paragraph, Wrap};

use crate::backend::catalogue::Device;
use crate::backend::fetch::Verification;
use crate::backend::preferences::Preferences;
use crate::backend::run::{Activity, Job, Obtained, RunError};
use crate::frontend::{preferences, select};

pub struct Model {
    select: select::Model,
    choosing: Option<Choosing>,
    confirming: Option<Confirming>,
    output_dir: PathBuf,
    run: Option<Run>,
}

/// A chosen device whose preferences are being picked before its image is obtained.
struct Choosing {
    device: &'static Device,
    menus: preferences::Model,
}

/// A chosen device whose complete preferences are shown for the operator to confirm before anything is downloaded.
struct Confirming {
    device: &'static Device,
    menus: preferences::Model,
    preferences: Preferences,
}

enum Run {
    Active { job: Job, preferences: Preferences },
    Ended { outcome: String, log: String },
}

impl Model {
    pub fn new(output_dir: PathBuf) -> Self {
        Self {
            select: select::Model::default(),
            choosing: None,
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
            if let Some(confirming) = model.confirming.take() {
                return confirm(model, confirming, key);
            }
            if interrupt {
                return true;
            }
            if let Some(mut choosing) = model.choosing.take() {
                match key.code {
                    KeyCode::Enter => match choosing.menus.confirmed() {
                        Some(preferences) => {
                            model.confirming = Some(Confirming {
                                device: choosing.device,
                                menus: choosing.menus,
                                preferences,
                            });
                        }
                        None => model.choosing = Some(choosing),
                    },
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

/// Applies a key on the confirmation view: Enter starts the run, Esc returns to the preferences with their choices
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
        KeyCode::Esc => {
            model.choosing = Some(Choosing {
                device: confirming.device,
                menus: confirming.menus,
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

pub fn view(model: &Model, frame: &mut Frame) {
    match &model.run {
        None => match (&model.confirming, &model.choosing) {
            (Some(confirming), _) => view_confirming(confirming, frame),
            (None, Some(choosing)) => preferences::view(&choosing.menus, frame),
            (None, None) => select::view(&model.select, frame),
        },
        Some(Run::Active { job, preferences }) => view_active(job.current(), preferences, frame),
        Some(Run::Ended { outcome, log }) => view_ended(outcome, log, frame),
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

fn view_active(activity: Activity, preferences: &Preferences, frame: &mut Frame) {
    let [status_area, progress_area, preferences_area, hint_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

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

fn view_confirming(confirming: &Confirming, frame: &mut Frame) {
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
    lines.push(
        "Press Enter to download and verify the image. Esc goes back to change your choices. Ctrl-C quits.".to_owned(),
    );

    frame.render_widget(
        Paragraph::new(lines.join("\n")).wrap(Wrap { trim: true }),
        frame.area(),
    );
}

fn view_ended(outcome: &str, log: &str, frame: &mut Frame) {
    let [outcome_area, log_area] =
        Layout::vertical([Constraint::Length(4), Constraint::Fill(1)]).areas(frame.area());

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
    fn enter_chooses_a_device_without_starting_it_and_escape_quits() {
        let mut model = Model::new(PathBuf::from("."));

        assert!(!update(
            &mut model,
            &key(KeyCode::Enter, KeyModifiers::NONE)
        ));
        assert!(model.run.is_none(), "choosing a device starts nothing");
        assert!(model.select.chosen().is_some());

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
        let mut model = Model::new(PathBuf::from("."));
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
    fn complete_preferences_show_a_confirmation_that_starts_nothing() {
        let mut model = Model::new(PathBuf::from("."));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));

        assert!(model.confirming.is_some(), "the confirmation is shown");
        assert!(model.choosing.is_none());
        assert!(model.run.is_none(), "confirming starts nothing");
    }

    #[test]
    fn escape_on_the_confirmation_returns_to_the_preferences_and_keeps_them() {
        let mut model = Model::new(PathBuf::from("."));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));

        assert!(!update(&mut model, &key(KeyCode::Esc, KeyModifiers::NONE)));

        assert!(model.confirming.is_none());
        let choosing = model.choosing.as_ref().expect("back on the preferences");
        assert_eq!(
            choosing.menus.confirmed().map(|p| p.harnesses),
            Some(vec![Harness::Pi])
        );
        assert!(model.run.is_none());
    }

    #[test]
    fn the_confirmation_shows_the_choices_before_the_download() {
        let mut model = Model::new(PathBuf::from("."));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Char(' '), KeyModifiers::NONE));
        update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE));

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
    }

    #[test]
    fn control_c_quits_before_a_run() {
        let mut model = Model::new(PathBuf::from("."));

        assert!(update(
            &mut model,
            &key(KeyCode::Char('c'), KeyModifiers::CONTROL)
        ));
    }
}
