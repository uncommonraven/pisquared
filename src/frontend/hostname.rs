//! The Hostname page of the Credentials section: the name the Pi answers to on the network, and the password of its DietPi
//! login. Up and down move between the two fields, and typing edits the field under the cursor. The rules are the
//! backend's, and the page continues only once both fields are valid. The password is masked until Tab shows it, and the
//! toggle resets when the page is left.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};

use crate::backend::network;

const DEFAULT_NAME: &str = "pisquared";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Field {
    #[default]
    Hostname,
    Password,
}

impl Field {
    const ALL: [Self; 2] = [Self::Hostname, Self::Password];

    fn label(self) -> &'static str {
        match self {
            Self::Hostname => "Hostname",
            Self::Password => "Password",
        }
    }
}

pub struct Model {
    field: Field,
    name: String,
    password: String,
    reveal: bool,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            field: Field::default(),
            name: DEFAULT_NAME.to_owned(),
            password: String::new(),
            reveal: false,
        }
    }
}

impl Model {
    /// The hostname, once it is valid.
    pub fn name(&self) -> Option<String> {
        self.error(Field::Hostname)
            .is_none()
            .then(|| self.name.clone())
    }

    /// The password, once it is valid.
    pub fn password(&self) -> Option<String> {
        self.error(Field::Password)
            .is_none()
            .then(|| self.password.clone())
    }

    /// Hides the password again, as it is whenever the page is left.
    pub fn leave(&mut self) {
        self.reveal = false;
    }

    fn moved(&self, step: isize) -> Field {
        let index = Field::ALL
            .iter()
            .position(|field| *field == self.field)
            .unwrap_or(0);
        Field::ALL[index.saturating_add_signed(step).min(Field::ALL.len() - 1)]
    }

    fn error(&self, field: Field) -> Option<String> {
        match field {
            Field::Hostname => network::validate_hostname(&self.name)
                .err()
                .map(|err| err.to_string()),
            Field::Password => network::validate_password(&self.password)
                .err()
                .map(|err| err.to_string()),
        }
    }

    fn text(&mut self) -> &mut String {
        match self.field {
            Field::Hostname => &mut self.name,
            Field::Password => &mut self.password,
        }
    }

    fn shown(&self, field: Field) -> String {
        match field {
            Field::Hostname => self.name.clone(),
            Field::Password if self.reveal => self.password.clone(),
            Field::Password => "*".repeat(self.password.chars().count()),
        }
    }
}

/// Applies a key press: Up and down move between the fields, Tab shows or hides the password, Backspace removes the last
/// character of the field under the cursor, and typing adds to it.
pub fn update(model: &mut Model, key: &KeyEvent) {
    match key.code {
        KeyCode::Up => model.field = model.moved(-1),
        KeyCode::Down => model.field = model.moved(1),
        KeyCode::Tab => model.reveal = !model.reveal,
        KeyCode::Backspace => {
            model.text().pop();
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => model.text().push(c),
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
                    .title("Hostname")
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
    }
    lines.push(Line::default());
    lines.push(Line::styled(
        "Hostname: letters, digits, and hyphens, up to 63 characters, and not starting or ending with a hyphen.",
        Style::new().dim(),
    ));
    lines.push(Line::styled(
        "Password: 8 to 63 letters, digits, or - _ . , ! @ % + : ^. It is the login password for the Pi, and it is stored in the image in plain text.",
        Style::new().dim(),
    ));
    lines
}

fn footer(model: &Model) -> String {
    let status = if model.name().is_some() && model.password().is_some() {
        "Enter continues to confirm your choices."
    } else {
        "Fix the fields to continue."
    };
    format!(
        "{status}\nUp and down move, type to edit, Tab shows or hides the password. Enter continues. Left or Esc goes back. Ctrl-C quits."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_text(model: &mut Model, text: &str) {
        for c in text.chars() {
            update(model, &key(KeyCode::Char(c)));
        }
    }

    /// A model with a valid password, typed after the default hostname.
    fn filled() -> Model {
        let mut model = Model::default();
        update(&mut model, &key(KeyCode::Down));
        type_text(&mut model, "pi-squared-2026");
        model
    }

    fn screen(model: &Model) -> String {
        let mut terminal = Terminal::new(TestBackend::new(120, 14)).unwrap();
        terminal
            .draw(|frame| view(model, frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..14)
            .map(|y| {
                (0..120)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_hostname_starts_as_pisquared_and_is_given_while_valid() {
        let model = filled();

        assert_eq!(model.name(), Some("pisquared".to_owned()));
        assert!(screen(&model).contains("Hostname: pisquared"));
    }

    #[test]
    fn typing_and_backspace_edit_the_hostname() {
        let mut model = Model::default();
        for _ in 0..9 {
            update(&mut model, &key(KeyCode::Backspace));
        }
        type_text(&mut model, "mypi");

        assert_eq!(model.name(), Some("mypi".to_owned()));
    }

    #[test]
    fn down_moves_to_the_password_and_typing_edits_it() {
        let mut model = Model::default();
        update(&mut model, &key(KeyCode::Down));
        type_text(&mut model, "pi-squared-2026");
        update(&mut model, &key(KeyCode::Up));
        type_text(&mut model, "x");

        assert_eq!(model.name, "pisquaredx");
        assert_eq!(model.password, "pi-squared-2026");
    }

    #[test]
    fn the_cursor_stops_at_the_first_and_last_fields() {
        let mut model = Model::default();
        update(&mut model, &key(KeyCode::Up));
        assert_eq!(model.field, Field::Hostname);

        update(&mut model, &key(KeyCode::Down));
        update(&mut model, &key(KeyCode::Down));
        assert_eq!(model.field, Field::Password);
    }

    #[test]
    fn an_empty_or_invalid_hostname_is_not_given_and_shows_its_error() {
        let mut model = Model::default();
        for _ in 0..9 {
            update(&mut model, &key(KeyCode::Backspace));
        }
        assert_eq!(model.name(), None);
        assert!(screen(&model).contains("the hostname is empty"));

        type_text(&mut model, "-");
        assert_eq!(model.name(), None);
        assert!(screen(&model).contains("cannot start or end with a hyphen"));
    }

    #[test]
    fn a_missing_or_short_password_is_not_given_and_shows_its_error() {
        let mut model = Model::default();
        assert_eq!(model.password(), None);
        assert!(screen(&model).contains("the password is shorter than 8 characters"));

        update(&mut model, &key(KeyCode::Down));
        type_text(&mut model, "1234567");
        assert_eq!(model.password(), None);

        type_text(&mut model, "8");
        assert_eq!(model.password(), Some("12345678".to_owned()));
    }

    #[test]
    fn the_password_is_masked_until_tab_shows_it_and_leave_hides_it_again() {
        let mut model = filled();
        assert!(!screen(&model).contains("pi-squared-2026"));
        assert!(screen(&model).contains("**************"));

        update(&mut model, &key(KeyCode::Tab));
        assert!(screen(&model).contains("pi-squared-2026"));

        model.leave();
        assert!(!screen(&model).contains("pi-squared-2026"));
    }

    #[test]
    fn control_characters_are_not_typed_into_a_field() {
        let mut model = Model::default();
        update(
            &mut model,
            &KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
        );

        assert_eq!(model.name(), Some("pisquared".to_owned()));
    }
}
