//! The Mesh Network page of the Credentials section: the auth key the Pi uses to join the operator's Mesh Network on its
//! first boot. Typing edits the key, and the page continues only once the key is valid. The key is masked until Tab shows
//! it, and the toggle resets when the page is left. The rules are the backend's.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};

use crate::backend::mesh_network;

#[derive(Default)]
pub struct Model {
    key: String,
    reveal: bool,
}

impl Model {
    /// The auth key, once it is valid.
    pub fn key(&self) -> Option<String> {
        self.error().is_none().then(|| self.key.clone())
    }

    /// Hides the key again, as it is whenever the page is left.
    pub fn leave(&mut self) {
        self.reveal = false;
    }

    fn error(&self) -> Option<String> {
        mesh_network::validate_auth_key(&self.key)
            .err()
            .map(|err| err.to_string())
    }

    fn shown(&self) -> String {
        if self.reveal {
            self.key.clone()
        } else {
            "*".repeat(self.key.chars().count())
        }
    }
}

/// Applies a key press: Tab shows or hides the key, Backspace removes its last character, and typing adds to it.
pub fn update(model: &mut Model, key: &KeyEvent) {
    match key.code {
        KeyCode::Tab => model.reveal = !model.reveal,
        KeyCode::Backspace => {
            model.key.pop();
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => model.key.push(c),
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
                    .title("Mesh Network")
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
    let mut lines = vec![Line::from(vec![
        Span::styled("> Auth key: ", Style::new().bold()),
        Span::raw(model.shown()),
    ])];
    if let Some(error) = model.error() {
        lines.push(Line::styled(format!("    {error}"), Style::new().red()));
    }
    lines.push(Line::default());
    lines.push(Line::styled(
        "Auth key: a Tailscale auth key, starting with tskey-. The Pi uses it once, on its first boot, to join your Mesh Network.",
        Style::new().dim(),
    ));
    lines.push(Line::styled(
        "Warning: the auth key is stored in plain text on the SD card until the Pi first boots. Anyone who reads the card before then can see it.",
        Style::new().yellow(),
    ));
    lines
}

fn footer(model: &Model) -> String {
    let status = if model.key().is_some() {
        "Enter continues to confirm your choices."
    } else {
        "Enter the auth key to continue."
    };
    format!(
        "{status}\nType to edit, Tab shows or hides the key. Enter continues. Left or Esc goes back. Ctrl-C quits."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const KEY: &str = "tskey-auth-kABC123-xyz789";

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
        type_text(&mut model, KEY);
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

    /// Wrapping breaks long text across rows, and the page's border runs down them, so the screen is compared by its words.
    fn words(text: &str) -> String {
        text.replace('│', " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn a_valid_key_is_given() {
        assert_eq!(filled().key(), Some(KEY.to_owned()));
    }

    #[test]
    fn an_empty_or_invalid_key_is_not_given_and_shows_its_error() {
        let mut model = Model::default();
        assert_eq!(model.key(), None);
        assert!(screen(&model).contains("the auth key is empty"));

        type_text(&mut model, "auth-kABC");
        assert_eq!(model.key(), None);
        assert!(screen(&model).contains("must start with tskey-"));
    }

    #[test]
    fn whitespace_and_quotes_are_rejected_with_their_errors() {
        let mut model = Model::default();
        type_text(&mut model, "tskey-ab c");
        assert_eq!(model.key(), None);
        assert!(screen(&model).contains("cannot contain whitespace"));

        update(&mut model, &key(KeyCode::Backspace));
        update(&mut model, &key(KeyCode::Backspace));
        update(&mut model, &key(KeyCode::Backspace));
        type_text(&mut model, "'");
        assert_eq!(model.key(), None);
        assert!(screen(&model).contains("cannot contain quote characters"));
    }

    #[test]
    fn the_key_is_masked_until_tab_shows_it_and_leave_hides_it_again() {
        let mut model = filled();
        assert!(!words(&screen(&model)).contains(KEY));
        assert!(words(&screen(&model)).contains(&"*".repeat(KEY.len())));

        update(&mut model, &key(KeyCode::Tab));
        assert!(words(&screen(&model)).contains(KEY));

        model.leave();
        assert!(!words(&screen(&model)).contains(KEY));
    }

    #[test]
    fn an_error_never_echoes_the_key() {
        let mut model = Model::default();
        type_text(&mut model, "secret-value");

        let error = model.error().expect("the key is invalid");
        assert!(error.contains("must start with tskey-"), "{error}");
        assert!(!error.contains("secret-value"), "{error}");
    }

    #[test]
    fn the_plain_text_warning_is_always_shown() {
        let rendered = words(&screen(&Model::default()));

        assert!(
            rendered.contains("stored in plain text on the SD card"),
            "{rendered}"
        );
    }

    #[test]
    fn backspace_edits_the_key_and_control_characters_are_not_typed() {
        let mut model = filled();
        update(&mut model, &key(KeyCode::Backspace));
        assert_eq!(model.key, &KEY[..KEY.len() - 1]);

        update(
            &mut model,
            &KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
        );
        assert_eq!(model.key, &KEY[..KEY.len() - 1]);
    }
}
