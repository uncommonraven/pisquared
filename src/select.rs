//! The device selection view: a filter over the catalogue, the device under the cursor, and the image it resolves to.
//!
//! Choosing a device fetches nothing. The view shows the image's full address and every caveat that attaches to it
//! before any download is started, so the operator sees what they are about to obtain. The decisions here are plain
//! values; the terminal only renders them.

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Style;
use ratatui::widgets::{Block, List, ListState, Paragraph, Wrap};

use crate::catalogue::{self, DEVICES, Device};

#[derive(Debug, Default)]
pub struct Model {
    filter: String,
    cursor: usize,
    chosen: Option<&'static Device>,
}

impl Model {
    /// The catalogue entries whose name or identifier contains the filter, ignoring case, in catalogue order.
    pub fn matches(&self) -> Vec<&'static Device> {
        let filter = self.filter.to_lowercase();
        DEVICES
            .iter()
            .filter(|d| d.name.to_lowercase().contains(&filter) || d.id.contains(&filter))
            .collect()
    }

    /// The device under the cursor, if the filter matches any.
    pub fn highlighted(&self) -> Option<&'static Device> {
        self.matches().get(self.cursor).copied()
    }

    /// The device the operator confirmed with Enter. Moving the cursor or changing the filter withdraws it.
    pub fn chosen(&self) -> Option<&'static Device> {
        self.chosen
    }
}

pub fn update(model: &mut Model, event: &Event) {
    let Event::Key(key) = event else {
        return;
    };
    if key.kind != KeyEventKind::Press {
        return;
    }
    match key.code {
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            model.filter.push(c);
            model.cursor = 0;
            model.chosen = None;
        }
        KeyCode::Backspace => {
            model.filter.pop();
            model.cursor = 0;
            model.chosen = None;
        }
        KeyCode::Up => {
            model.cursor = model.cursor.saturating_sub(1);
            model.chosen = None;
        }
        KeyCode::Down => {
            let last = model.matches().len().saturating_sub(1);
            model.cursor = (model.cursor + 1).min(last);
            model.chosen = None;
        }
        KeyCode::Enter => model.chosen = model.highlighted(),
        _ => {}
    }
}

pub fn view(model: &Model, frame: &mut Frame) {
    let [filter_area, list_area, detail_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(10),
    ])
    .areas(frame.area());

    frame.render_widget(
        Paragraph::new(format!("Filter: {}", model.filter)),
        filter_area,
    );

    let matches = model.matches();
    let names: Vec<&str> = matches.iter().map(|d| d.name).collect();
    let mut state = ListState::default();
    if !matches.is_empty() {
        state.select(Some(model.cursor));
    }
    let list = List::new(names)
        .block(Block::bordered().title("Devices (up/down to move, Enter to choose, Esc to quit)"))
        .highlight_style(Style::new().reversed())
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, list_area, &mut state);

    frame.render_widget(
        Paragraph::new(detail(model)).wrap(Wrap { trim: true }),
        detail_area,
    );
}

fn detail(model: &Model) -> String {
    let Some(device) = model.highlighted() else {
        return "No device matches the filter, so no image is chosen.".to_owned();
    };

    let resolution =
        catalogue::resolve(device.id).expect("the highlighted device is in the catalogue");
    let status = match model.chosen() {
        Some(chosen) => format!(
            "Chosen: {}. Nothing has been fetched. Press Enter again to start the download.",
            chosen.name
        ),
        None => "Press Enter to choose this device. Choosing fetches nothing.".to_owned(),
    };
    let mut lines = vec![
        status,
        format!("Device: {}", resolution.device.name),
        format!("Image: {}", resolution.device.image_name()),
        format!("Address: {}", resolution.address),
    ];
    lines.extend(
        resolution
            .caveats
            .iter()
            .map(|caveat| format!("Caveat: {caveat}")),
    );
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::CAVEAT_RPI234_PI5;
    use crossterm::event::KeyEvent;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const WIDTH: u16 = 120;
    const HEIGHT: u16 = 30;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn type_text(model: &mut Model, text: &str) {
        for c in text.chars() {
            update(model, &key(KeyCode::Char(c)));
        }
    }

    fn ids(model: &Model) -> Vec<&'static str> {
        model.matches().iter().map(|d| d.id).collect()
    }

    fn screen(model: &Model) -> String {
        let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
        terminal.draw(|frame| view(model, frame)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..HEIGHT)
            .map(|y| {
                (0..WIDTH)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Wrapping breaks long text across rows, so the screen is compared by its words.
    fn words(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn an_empty_filter_lists_every_device() {
        assert_eq!(Model::default().matches().len(), DEVICES.len());
    }

    #[test]
    fn the_filter_matches_names_and_identifiers_ignoring_case() {
        let mut model = Model::default();
        type_text(&mut model, "COMPUTE");
        assert_eq!(ids(&model), ["cm-1", "cm-4", "cm-5"]);

        for _ in 0.."COMPUTE".len() {
            update(&mut model, &key(KeyCode::Backspace));
        }
        type_text(&mut model, "rpi-4");
        assert_eq!(ids(&model), ["rpi-4", "rpi-400"]);
    }

    #[test]
    fn nothing_is_chosen_until_enter() {
        let mut model = Model::default();
        type_text(&mut model, "compute module 4");
        assert!(model.chosen().is_none());

        update(&mut model, &key(KeyCode::Enter));

        assert_eq!(model.chosen().map(|d| d.id), Some("cm-4"));
    }

    #[test]
    fn moving_the_cursor_withdraws_the_choice_and_stops_at_the_ends() {
        let mut model = Model::default();
        update(&mut model, &key(KeyCode::Enter));
        update(&mut model, &key(KeyCode::Up));

        assert_eq!(model.highlighted().map(|d| d.id), Some("rpi-1"));
        assert!(model.chosen().is_none());

        for _ in 0..DEVICES.len() + 5 {
            update(&mut model, &key(KeyCode::Down));
        }
        assert_eq!(model.highlighted().map(|d| d.id), Some("cm-5"));
    }

    #[test]
    fn a_filter_with_no_match_chooses_nothing() {
        let mut model = Model::default();
        type_text(&mut model, "pico");
        update(&mut model, &key(KeyCode::Enter));

        assert!(model.chosen().is_none());
        assert!(screen(&model).contains("No device matches the filter"));
    }

    #[test]
    fn control_chords_do_not_type_into_the_filter() {
        let mut model = Model::default();
        let chord = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

        update(&mut model, &chord);

        assert_eq!(model.filter, "");
    }

    /// Task 7.2: the view names the chosen image in full, with its address, and nothing else's image.
    #[test]
    fn the_rendered_view_names_the_chosen_image_in_full() {
        let mut model = Model::default();
        type_text(&mut model, "raspberry pi 5");
        update(&mut model, &key(KeyCode::Enter));

        let rendered = words(&screen(&model));

        assert_eq!(model.chosen().map(|d| d.id), Some("rpi-5"));
        assert!(rendered.contains("Chosen: Raspberry Pi 5. Nothing has been fetched."));
        assert!(rendered.contains("Image: DietPi_RPi5-ARMv8-Trixie.img.xz"));
        assert!(
            rendered.contains(
                "Address: https://dietpi.com/downloads/images/DietPi_RPi5-ARMv8-Trixie.img.xz"
            ),
            "{rendered}"
        );
        assert!(!rendered.contains("RPi234"), "{rendered}");
        assert!(!rendered.contains("Caveat:"), "{rendered}");
    }

    /// Task 7.2: a caveat reaches the display for the device it attaches to, before the device is chosen.
    #[test]
    fn a_caveat_reaches_the_display_before_anything_is_chosen() {
        let mut model = Model::default();
        type_text(&mut model, "compute module 4");

        let rendered = words(&screen(&model));

        assert!(model.chosen().is_none());
        assert!(
            rendered.contains(
                "Address: https://dietpi.com/downloads/images/DietPi_RPi234-ARMv8-Trixie.img.xz"
            ),
            "{rendered}"
        );
        assert!(
            rendered.contains(&words(&format!("Caveat: {CAVEAT_RPI234_PI5}"))),
            "{rendered}"
        );
    }
}
