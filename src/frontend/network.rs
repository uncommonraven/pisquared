//! The Network page of the Credentials section: the connection, Wi-Fi or Ethernet, and for Wi-Fi the network name, its
//! passphrase, and its country code.
//!
//! Up and down move between the fields. Space or Right switches the connection while its field is under the cursor,
//! and typing edits the field under the cursor. Ethernet hides the Wi-Fi fields, since it needs nothing else. Every
//! visible field shows its error next to it, and the page continues only once all of them are valid. The passphrase is
//! masked until Tab shows it. The country code is searched by typing: the countries whose code or name matches are
//! listed beneath the field, and the code is kept in capitals. The rules are the backend's; this view holds only what
//! is typed and where the cursor is.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};

use crate::backend::network::{self, Connection, Wifi};

/// How many matching countries are listed beneath the country code.
const MATCHES_SHOWN: usize = 5;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Field {
    #[default]
    Connection,
    Ssid,
    Passphrase,
    Country,
}

impl Field {
    const ALL: [Self; 4] = [
        Self::Connection,
        Self::Ssid,
        Self::Passphrase,
        Self::Country,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Connection => "Connection",
            Self::Ssid => "Wi-Fi network name (SSID)",
            Self::Passphrase => "Wi-Fi passphrase",
            Self::Country => "Wi-Fi country code",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Medium {
    #[default]
    Wifi,
    Ethernet,
}

impl Medium {
    fn toggled(self) -> Self {
        match self {
            Self::Wifi => Self::Ethernet,
            Self::Ethernet => Self::Wifi,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Wifi => "Wi-Fi",
            Self::Ethernet => "Ethernet",
        }
    }
}

#[derive(Default)]
pub struct Model {
    field: Field,
    medium: Medium,
    ssid: String,
    passphrase: String,
    country_code: String,
    reveal: bool,
}

impl Model {
    /// The connection, once its fields are valid. Ethernet needs nothing, and Wi-Fi needs every Wi-Fi field.
    pub fn connection(&self) -> Option<Connection> {
        match self.medium {
            Medium::Ethernet => Some(Connection::Ethernet),
            Medium::Wifi => self.wifi().map(Connection::Wifi),
        }
    }

    /// Hides the passphrase again, as it is whenever the page is left.
    pub fn leave(&mut self) {
        self.reveal = false;
    }

    fn wifi(&self) -> Option<Wifi> {
        [Field::Ssid, Field::Passphrase, Field::Country]
            .iter()
            .all(|field| self.error(*field).is_none())
            .then(|| Wifi {
                ssid: self.ssid.clone(),
                passphrase: self.passphrase.clone(),
                country_code: self.country_code.clone(),
            })
    }

    fn visible(&self) -> &'static [Field] {
        match self.medium {
            Medium::Wifi => &Field::ALL,
            Medium::Ethernet => &[Field::Connection],
        }
    }

    fn moved(&self, step: isize) -> Field {
        let visible = self.visible();
        let index = visible
            .iter()
            .position(|field| *field == self.field)
            .unwrap_or(0);
        visible[index.saturating_add_signed(step).min(visible.len() - 1)]
    }

    fn error(&self, field: Field) -> Option<String> {
        match field {
            Field::Connection => None,
            Field::Ssid => network::validate_ssid(&self.ssid)
                .err()
                .map(|err| err.to_string()),
            Field::Passphrase => network::validate_passphrase(&self.passphrase)
                .err()
                .map(|err| err.to_string()),
            Field::Country => network::validate_country_code(&self.country_code)
                .err()
                .map(|err| err.to_string()),
        }
    }

    fn text(&mut self) -> Option<&mut String> {
        match self.field {
            Field::Connection => None,
            Field::Ssid => Some(&mut self.ssid),
            Field::Passphrase => Some(&mut self.passphrase),
            Field::Country => Some(&mut self.country_code),
        }
    }

    fn shown(&self, field: Field) -> String {
        match field {
            Field::Connection => self.medium.name().to_owned(),
            Field::Ssid => self.ssid.clone(),
            Field::Passphrase if self.reveal => self.passphrase.clone(),
            Field::Passphrase => "*".repeat(self.passphrase.chars().count()),
            Field::Country => match network::country_name(&self.country_code) {
                Some(name) => format!("{} ({name})", self.country_code),
                None => self.country_code.clone(),
            },
        }
    }
}

/// Applies a key press: Up and down move between the fields, Space or Right switches the connection, Tab shows or hides
/// the passphrase, and typing edits the field under the cursor.
pub fn update(model: &mut Model, key: &KeyEvent) {
    match key.code {
        KeyCode::Up => model.field = model.moved(-1),
        KeyCode::Down => model.field = model.moved(1),
        KeyCode::Tab => model.reveal = !model.reveal,
        KeyCode::Right | KeyCode::Char(' ') if model.field == Field::Connection => {
            model.medium = model.medium.toggled();
        }
        KeyCode::Backspace => {
            if let Some(text) = model.text() {
                text.pop();
            }
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let c = match model.field {
                Field::Country => c.to_ascii_uppercase(),
                _ => c,
            };
            if let Some(text) = model.text() {
                text.push(c);
            }
        }
        _ => {}
    }
}

pub fn view(model: &Model, frame: &mut Frame, area: Rect) {
    let [page_area, footer_area] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(3)]).areas(area);

    frame.render_widget(
        Paragraph::new(page_lines(model))
            .wrap(Wrap { trim: false })
            .block(
                Block::bordered()
                    .title("Network")
                    .border_style(Style::new().bold()),
            ),
        page_area,
    );
    frame.render_widget(
        Paragraph::new(footer(model)).wrap(Wrap { trim: true }),
        footer_area,
    );
}

fn page_lines(model: &Model) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for &field in model.visible() {
        let focused = field == model.field;
        let marker = if focused { "> " } else { "  " };
        let label_style = if focused {
            Style::new().bold()
        } else {
            Style::new()
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{marker}{}: ", field.label()), label_style),
            Span::raw(model.shown(field)),
        ]));
        if let Some(error) = model.error(field) {
            lines.push(Line::styled(format!("    {error}"), Style::new().red()));
        }
        if focused && field == Field::Country {
            lines.extend(matching_lines(&model.country_code));
        }
    }
    lines.push(Line::default());
    lines.push(Line::styled(warning(model), Style::new().yellow()));
    lines
}

fn warning(model: &Model) -> &'static str {
    match model.medium {
        Medium::Wifi => {
            "Warning: a wrong SSID or passphrase leaves the Pi offline. Ethernet is disabled, so there is no fallback."
        }
        Medium::Ethernet => {
            "Warning: with Ethernet chosen there is no Wi-Fi fallback, so the Pi stays offline unless its cable reaches a network."
        }
    }
}

fn matching_lines(query: &str) -> Vec<Line<'static>> {
    let matches = network::countries_matching(query);
    let dim = Style::new().dim();
    let mut lines: Vec<Line<'static>> = matches
        .iter()
        .take(MATCHES_SHOWN)
        .map(|(code, name)| Line::styled(format!("      {code}  {name}"), dim))
        .collect();
    if matches.len() > MATCHES_SHOWN {
        lines.push(Line::styled(
            format!("      and {} more", matches.len() - MATCHES_SHOWN),
            dim,
        ));
    }
    lines
}

fn footer(model: &Model) -> String {
    let status = if model.connection().is_some() {
        "Enter continues to confirm your choices."
    } else {
        "Fill in every field to continue."
    };
    format!(
        "{status}\nUp and down move, Space or Right switches Wi-Fi and Ethernet, type to edit, Tab shows or hides the passphrase. Enter continues. Left or Esc goes back. Ctrl-C quits."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const WIDTH: u16 = 120;
    const HEIGHT: u16 = 16;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_text(model: &mut Model, text: &str) {
        for c in text.chars() {
            update(model, &key(KeyCode::Char(c)));
        }
    }

    /// A valid Wi-Fi network, entered from the first Wi-Fi field.
    fn filled() -> Model {
        let mut model = Model::default();
        update(&mut model, &key(KeyCode::Down));
        type_text(&mut model, "Home");
        update(&mut model, &key(KeyCode::Down));
        type_text(&mut model, "correct horse");
        update(&mut model, &key(KeyCode::Down));
        type_text(&mut model, "gb");
        model
    }

    fn screen_sized(model: &Model, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| view(model, frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn screen(model: &Model) -> String {
        screen_sized(model, WIDTH, HEIGHT)
    }

    /// Wrapping breaks long text across rows, so the screen is compared by its words.
    fn words(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn the_connection_starts_on_wifi_and_space_switches_it() {
        let mut model = Model::default();
        assert_eq!(model.medium, Medium::Wifi);

        update(&mut model, &key(KeyCode::Char(' ')));
        assert_eq!(model.medium, Medium::Ethernet);

        update(&mut model, &key(KeyCode::Right));
        assert_eq!(model.medium, Medium::Wifi);
    }

    #[test]
    fn typing_on_the_connection_field_edits_nothing() {
        let mut model = Model::default();
        type_text(&mut model, "Home");

        assert_eq!(model.ssid, "");
        assert_eq!(model.medium, Medium::Wifi);
    }

    #[test]
    fn typing_edits_the_field_under_the_cursor() {
        let mut model = Model::default();
        update(&mut model, &key(KeyCode::Down));
        type_text(&mut model, "Home");
        update(&mut model, &key(KeyCode::Down));
        type_text(&mut model, "pass");
        update(&mut model, &key(KeyCode::Up));
        update(&mut model, &key(KeyCode::Backspace));

        assert_eq!(model.ssid, "Hom");
        assert_eq!(model.passphrase, "pass");
        assert_eq!(model.country_code, "");
    }

    #[test]
    fn the_cursor_stops_at_the_first_and_last_fields() {
        let mut model = Model::default();
        update(&mut model, &key(KeyCode::Up));
        assert_eq!(model.field, Field::Connection);

        for _ in 0..5 {
            update(&mut model, &key(KeyCode::Down));
        }
        assert_eq!(model.field, Field::Country);
    }

    #[test]
    fn country_codes_are_kept_in_capitals() {
        let mut model = Model::default();
        for _ in 0..3 {
            update(&mut model, &key(KeyCode::Down));
        }
        type_text(&mut model, "gb");

        assert_eq!(model.country_code, "GB");
    }

    #[test]
    fn ctrl_characters_are_not_typed_into_a_field() {
        let mut model = Model::default();
        update(&mut model, &key(KeyCode::Down));
        update(
            &mut model,
            &KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
        );

        assert_eq!(model.ssid, "");
    }

    #[test]
    fn the_network_is_given_only_once_every_field_is_valid() {
        let mut model = Model::default();
        assert_eq!(model.connection(), None);

        model.passphrase = "1234567".to_owned();
        model.ssid = "Home".to_owned();
        model.country_code = "GB".to_owned();
        assert_eq!(
            model.connection(),
            None,
            "a seven-character passphrase is too short"
        );

        model.passphrase = "12345678".to_owned();
        assert_eq!(
            model.connection(),
            Some(Connection::Wifi(Wifi {
                ssid: "Home".to_owned(),
                passphrase: "12345678".to_owned(),
                country_code: "GB".to_owned(),
            }))
        );
    }

    #[test]
    fn a_country_code_is_required_because_none_is_assumed() {
        let mut model = filled();
        model.country_code.clear();

        assert_eq!(model.connection(), None);
    }

    #[test]
    fn ethernet_needs_no_wifi_fields_and_hides_them() {
        let mut model = Model::default();
        update(&mut model, &key(KeyCode::Char(' ')));

        assert_eq!(model.connection(), Some(Connection::Ethernet));
        let rendered = words(&screen(&model));
        assert!(rendered.contains("Connection: Ethernet"), "{rendered}");
        assert!(!rendered.contains("Wi-Fi network name"), "{rendered}");
        assert!(!rendered.contains("Wi-Fi passphrase"), "{rendered}");
        assert!(
            !rendered.contains("the network name is empty"),
            "{rendered}"
        );
    }

    #[test]
    fn the_cursor_stays_on_the_connection_while_ethernet_is_chosen() {
        let mut model = Model::default();
        update(&mut model, &key(KeyCode::Char(' ')));
        update(&mut model, &key(KeyCode::Down));

        assert_eq!(model.field, Field::Connection);
    }

    #[test]
    fn switching_back_to_wifi_keeps_what_was_typed() {
        let mut model = filled();
        for _ in 0..3 {
            update(&mut model, &key(KeyCode::Up));
        }
        update(&mut model, &key(KeyCode::Char(' ')));
        assert_eq!(model.connection(), Some(Connection::Ethernet));

        update(&mut model, &key(KeyCode::Char(' ')));
        assert_eq!(
            model.connection(),
            Some(Connection::Wifi(Wifi {
                ssid: "Home".to_owned(),
                passphrase: "correct horse".to_owned(),
                country_code: "GB".to_owned(),
            }))
        );
    }

    #[test]
    fn the_passphrase_is_masked_until_tab_shows_it() {
        let mut model = filled();
        assert!(!words(&screen(&model)).contains("correct horse"));
        assert!(words(&screen(&model)).contains("*************"));

        update(&mut model, &key(KeyCode::Tab));
        assert!(words(&screen(&model)).contains("correct horse"));

        model.leave();
        assert!(!words(&screen(&model)).contains("correct horse"));
    }

    #[test]
    fn each_invalid_field_shows_its_error_and_the_warning_is_always_shown() {
        let rendered = words(&screen(&Model::default()));

        assert!(rendered.contains("the network name is empty"), "{rendered}");
        assert!(
            rendered.contains("Warning: a wrong SSID or passphrase leaves the Pi offline."),
            "{rendered}"
        );
        assert!(
            rendered.contains("Fill in every field to continue."),
            "{rendered}"
        );
    }

    #[test]
    fn the_warning_wraps_instead_of_running_off_a_narrow_page() {
        let rendered = screen_sized(&Model::default(), 60, 20).replace('│', " ");
        let rendered = words(&rendered);

        assert!(
            rendered.contains("Ethernet is disabled, so there is no fallback."),
            "{rendered}"
        );
    }

    #[test]
    fn a_country_code_lists_the_countries_it_matches_and_names_the_chosen_one() {
        let mut model = Model::default();
        for _ in 0..3 {
            update(&mut model, &key(KeyCode::Down));
        }
        type_text(&mut model, "united k");

        let rendered = words(&screen(&model));
        assert!(
            rendered.contains("GB United Kingdom of Great Britain and Northern Ireland"),
            "{rendered}"
        );

        model.country_code.clear();
        type_text(&mut model, "GB");
        assert!(words(&screen(&model)).contains("GB (United Kingdom"));
    }
}
