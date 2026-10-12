//! The Network page of the Credentials section: the Wi-Fi network name, its passphrase, and its country code.
//!
//! Up and down move between the fields, and typing edits the field under the cursor. Every field shows its error next
//! to it, and the page continues only once all of them are valid. The passphrase is masked until Tab shows it. The
//! country code is searched by typing: the countries whose code or name matches are listed beneath the field, and the
//! code is kept in capitals. The rules are the backend's; this view holds only what is typed and where the cursor is.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};

use crate::backend::network::{self, Wifi};

/// How many matching countries are listed beneath the country code.
const MATCHES_SHOWN: usize = 5;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Field {
    #[default]
    Ssid,
    Passphrase,
    Country,
}

impl Field {
    const ALL: [Self; 3] = [Self::Ssid, Self::Passphrase, Self::Country];

    fn label(self) -> &'static str {
        match self {
            Self::Ssid => "Wi-Fi network name (SSID)",
            Self::Passphrase => "Wi-Fi passphrase",
            Self::Country => "Wi-Fi country code",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Ssid => Self::Passphrase,
            Self::Passphrase | Self::Country => Self::Country,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Ssid | Self::Passphrase => Self::Ssid,
            Self::Country => Self::Passphrase,
        }
    }
}

#[derive(Default)]
pub struct Model {
    field: Field,
    ssid: String,
    passphrase: String,
    country_code: String,
    reveal: bool,
}

impl Model {
    /// The Wi-Fi network, once every field is valid.
    pub fn wifi(&self) -> Option<Wifi> {
        Field::ALL
            .iter()
            .all(|field| self.error(*field).is_none())
            .then(|| Wifi {
                ssid: self.ssid.clone(),
                passphrase: self.passphrase.clone(),
                country_code: self.country_code.clone(),
            })
    }

    /// Hides the passphrase again, as it is whenever the page is left.
    pub fn leave(&mut self) {
        self.reveal = false;
    }

    fn error(&self, field: Field) -> Option<String> {
        match field {
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

    fn text(&mut self) -> &mut String {
        match self.field {
            Field::Ssid => &mut self.ssid,
            Field::Passphrase => &mut self.passphrase,
            Field::Country => &mut self.country_code,
        }
    }

    fn shown(&self, field: Field) -> String {
        match field {
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

/// Applies a key press: Up and down move between the fields, Tab shows or hides the passphrase, and typing edits the
/// field under the cursor.
pub fn update(model: &mut Model, key: &KeyEvent) {
    match key.code {
        KeyCode::Up => model.field = model.field.previous(),
        KeyCode::Down => model.field = model.field.next(),
        KeyCode::Tab => model.reveal = !model.reveal,
        KeyCode::Backspace => {
            model.text().pop();
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let c = match model.field {
                Field::Country => c.to_ascii_uppercase(),
                _ => c,
            };
            model.text().push(c);
        }
        _ => {}
    }
}

pub fn view(model: &Model, frame: &mut Frame, area: Rect) {
    let [page_area, footer_area] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(3)]).areas(area);

    frame.render_widget(
        Paragraph::new(page_lines(model)).block(
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
    for field in Field::ALL {
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
    lines.push(Line::styled(
        "Warning: a wrong SSID or passphrase leaves the Pi offline. Ethernet is disabled, so there is no fallback.",
        Style::new().yellow(),
    ));
    lines
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
    let status = if model.wifi().is_some() {
        "Enter continues to confirm your choices."
    } else {
        "Fill in every field to continue."
    };
    format!(
        "{status}\nUp and down move, type to edit, Tab shows or hides the passphrase. Enter continues. Left or Esc goes back. Ctrl-C quits."
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

    fn filled() -> Model {
        let mut model = Model::default();
        type_text(&mut model, "Home");
        update(&mut model, &key(KeyCode::Down));
        type_text(&mut model, "correct horse");
        update(&mut model, &key(KeyCode::Down));
        type_text(&mut model, "gb");
        model
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
    fn typing_edits_the_field_under_the_cursor() {
        let mut model = Model::default();
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
        assert_eq!(model.field, Field::Ssid);

        for _ in 0..5 {
            update(&mut model, &key(KeyCode::Down));
        }
        assert_eq!(model.field, Field::Country);
    }

    #[test]
    fn country_codes_are_kept_in_capitals() {
        let mut model = Model::default();
        update(&mut model, &key(KeyCode::Down));
        update(&mut model, &key(KeyCode::Down));
        type_text(&mut model, "gb");

        assert_eq!(model.country_code, "GB");
    }

    #[test]
    fn ctrl_characters_are_not_typed_into_a_field() {
        let mut model = Model::default();
        update(
            &mut model,
            &KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
        );

        assert_eq!(model.ssid, "");
    }

    #[test]
    fn the_network_is_given_only_once_every_field_is_valid() {
        let mut model = Model::default();
        assert_eq!(model.wifi(), None);

        model.passphrase = "1234567".to_owned();
        model.ssid = "Home".to_owned();
        model.country_code = "GB".to_owned();
        assert_eq!(
            model.wifi(),
            None,
            "a seven-character passphrase is too short"
        );

        model.passphrase = "12345678".to_owned();
        assert_eq!(
            model.wifi(),
            Some(Wifi {
                ssid: "Home".to_owned(),
                passphrase: "12345678".to_owned(),
                country_code: "GB".to_owned(),
            })
        );
    }

    #[test]
    fn a_country_code_is_required_because_none_is_assumed() {
        let mut model = filled();
        model.country_code.clear();

        assert_eq!(model.wifi(), None);
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
    fn a_country_code_lists_the_countries_it_matches_and_names_the_chosen_one() {
        let mut model = Model::default();
        update(&mut model, &key(KeyCode::Down));
        update(&mut model, &key(KeyCode::Down));
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
