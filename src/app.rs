//! The interface as a whole: choosing a device, then obtaining its image while showing progress, then the outcome.
//!
//! Before a run starts the device selection view has the screen. While a run is in progress the screen shows what
//! it is doing and Esc or Ctrl-C cancels it. Once the run has ended the screen shows its outcome and its log, and
//! Esc or Ctrl-C quits.

use std::path::PathBuf;

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::widgets::{Block, Gauge, Paragraph, Wrap};

use crate::run::{Activity, Job, Obtained, RunError};
use crate::select;

pub struct Model {
    select: select::Model,
    output_dir: PathBuf,
    run: Option<Run>,
}

enum Run {
    Active(Job),
    Ended { outcome: String, log: String },
}

impl Model {
    pub fn new(output_dir: PathBuf) -> Self {
        Self {
            select: select::Model::default(),
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
        Some(Run::Active(job)) => {
            if interrupt {
                job.cancel();
            }
            false
        }
        Some(Run::Ended { .. }) => interrupt,
        None if interrupt => true,
        None => {
            if key.code == KeyCode::Enter
                && let Some(device) = model.select.chosen()
            {
                model.run = Some(Run::Active(Job::start(device.id, model.output_dir.clone())));
            } else {
                select::update(&mut model.select, event);
            }
            false
        }
    }
}

/// Takes in what an active run has reported, and moves to its outcome once the run has ended.
pub fn tick(model: &mut Model) {
    let Some(Run::Active(job)) = &mut model.run else {
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
        None => select::view(&model.select, frame),
        Some(Run::Active(job)) => view_active(job.current(), frame),
        Some(Run::Ended { outcome, log }) => view_ended(outcome, log, frame),
    }
}

fn outcome(result: &Result<Obtained, RunError>) -> String {
    match result {
        Ok(obtained) => format!(
            "Obtained {}. The download was verified; the decompressed file is not, since the publisher publishes no value for it.\nPress Esc to quit.",
            obtained.flashable.display()
        ),
        Err(err) => format!("{err}\nPress Esc to quit."),
    }
}

fn view_active(activity: Activity, frame: &mut Frame) {
    let [status_area, progress_area, hint_area] = Layout::vertical([
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
    frame.render_widget(
        Paragraph::new(
            "Press Esc or Ctrl-C to cancel. A cancelled run leaves nothing that looks finished.",
        ),
        hint_area,
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
    use crossterm::event::{KeyEvent, KeyModifiers};

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
    fn control_c_quits_before_a_run() {
        let mut model = Model::new(PathBuf::from("."));

        assert!(update(
            &mut model,
            &key(KeyCode::Char('c'), KeyModifiers::CONTROL)
        ));
    }
}
