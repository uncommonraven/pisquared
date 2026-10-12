//! The device selection screen: the catalogue as a radio list, the device under the cursor, and the image it resolves to.
//!
//! Up and down move the cursor, and Space picks the device under it, replacing any earlier pick. Enter continues to the
//! preferences once a device is picked. Choosing a device fetches nothing. The view shows the image's full address and
//! every caveat that attaches to it before any download is started, so the operator sees what they are about to obtain.
//! The decisions here are plain values; the terminal only renders them.

use crossterm::event::{Event, KeyCode, KeyEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, List, ListState, Paragraph, Wrap};

use crate::backend::catalogue::{DEVICES, Device};

#[derive(Debug, Default)]
pub struct Model {
    cursor: usize,
    chosen: Option<&'static Device>,
}

impl Model {
    /// The device under the cursor.
    pub fn highlighted(&self) -> Option<&'static Device> {
        DEVICES.get(self.cursor)
    }

    /// The device the operator picked with Space. Moving the cursor leaves it as it is.
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
        KeyCode::Up => model.cursor = model.cursor.saturating_sub(1),
        KeyCode::Down => {
            let last = DEVICES.len().saturating_sub(1);
            model.cursor = (model.cursor + 1).min(last);
        }
        KeyCode::Char(' ') => model.chosen = model.highlighted(),
        _ => {}
    }
}

pub fn view(model: &Model, frame: &mut Frame, area: Rect) {
    let [list_area, detail_area, footer_area] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(10),
        Constraint::Length(3),
    ])
    .areas(area);

    let items: Vec<String> = DEVICES
        .iter()
        .map(|device| {
            let mark = if model.chosen().is_some_and(|chosen| chosen.id == device.id) {
                "(•)"
            } else {
                "( )"
            };
            format!("{mark} {}", device.name)
        })
        .collect();
    let mut state = ListState::default();
    state.select(Some(model.cursor));
    let list = List::new(items)
        .block(
            Block::bordered()
                .title("Devices (choose one)")
                .border_style(Style::new().bold()),
        )
        .highlight_style(Style::new().reversed())
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, list_area, &mut state);

    frame.render_widget(
        Paragraph::new(detail(model)).wrap(Wrap { trim: true }),
        detail_area,
    );
    frame.render_widget(
        Paragraph::new(footer(model)).wrap(Wrap { trim: true }),
        footer_area,
    );
}

fn detail(model: &Model) -> String {
    let Some(device) = model.highlighted() else {
        return String::new();
    };

    let mut lines = vec![
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
    lines.join("\n")
}

fn footer(model: &Model) -> String {
    let status = match model.chosen() {
        Some(_) => "Nothing has been fetched. Enter continues to the preferences.".to_owned(),
        None => "Pick a device to continue.".to_owned(),
    };
    format!("{status}\nUp and down move, Space picks, Enter continues. Esc quits.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::catalogue::CAVEAT_RPI234_PI5;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const WIDTH: u16 = 120;
    const HEIGHT: u16 = 30;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    /// Moves the cursor to the device with the given identifier.
    fn highlight(model: &mut Model, id: &str) {
        model.cursor = DEVICES.iter().position(|d| d.id == id).unwrap();
    }

    fn screen(model: &Model) -> String {
        let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
        terminal
            .draw(|frame| view(model, frame, frame.area()))
            .unwrap();
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
    fn nothing_is_chosen_until_space() {
        let mut model = Model::default();
        highlight(&mut model, "cm-4");
        update(&mut model, &key(KeyCode::Enter));
        assert!(model.chosen().is_none(), "Enter does not pick");

        update(&mut model, &key(KeyCode::Char(' ')));

        assert_eq!(model.chosen().map(|d| d.id), Some("cm-4"));
    }

    #[test]
    fn picking_another_device_replaces_the_first() {
        let mut model = Model::default();
        highlight(&mut model, "cm-4");
        update(&mut model, &key(KeyCode::Char(' ')));
        highlight(&mut model, "rpi-5");
        update(&mut model, &key(KeyCode::Char(' ')));

        assert_eq!(model.chosen().map(|d| d.id), Some("rpi-5"));
    }

    #[test]
    fn moving_the_cursor_keeps_the_pick_and_stops_at_the_ends() {
        let mut model = Model::default();
        update(&mut model, &key(KeyCode::Char(' ')));
        update(&mut model, &key(KeyCode::Up));

        assert_eq!(model.highlighted().map(|d| d.id), Some(DEVICES[0].id));
        assert_eq!(model.chosen().map(|d| d.id), Some(DEVICES[0].id));

        for _ in 0..DEVICES.len() + 5 {
            update(&mut model, &key(KeyCode::Down));
        }
        assert_eq!(
            model.highlighted().map(|d| d.id),
            Some(DEVICES[DEVICES.len() - 1].id)
        );
    }

    /// The view names the chosen image in full, with its address, and nothing else's image.
    #[test]
    fn the_rendered_view_names_the_chosen_image_in_full() {
        let mut model = Model::default();
        highlight(&mut model, "rpi-5");
        update(&mut model, &key(KeyCode::Char(' ')));

        let rendered = words(&screen(&model));

        assert_eq!(model.chosen().map(|d| d.id), Some("rpi-5"));
        assert!(rendered.contains("(•) Raspberry Pi 5"), "{rendered}");
        assert!(rendered.contains("Image: DietPi_RPi5-ARMv8-Trixie.img.xz"));
        assert!(
            rendered.contains(
                "Address: https://dietpi.com/downloads/images/DietPi_RPi5-ARMv8-Trixie.img.xz"
            ),
            "{rendered}"
        );
        assert!(!rendered.contains("RPi234"), "{rendered}");
        assert!(!rendered.contains("Caveat:"), "{rendered}");
        assert!(
            rendered.contains("Nothing has been fetched. Enter continues to the preferences."),
            "{rendered}"
        );
    }

    /// A caveat reaches the display for the device it attaches to, before the device is chosen.
    #[test]
    fn a_caveat_reaches_the_display_before_anything_is_chosen() {
        let mut model = Model::default();
        highlight(&mut model, "cm-4");

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
        assert!(
            rendered.contains("Pick a device to continue."),
            "{rendered}"
        );
    }
}
