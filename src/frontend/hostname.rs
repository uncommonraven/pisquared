//! The Hostname page of the Credentials section: the name the Pi answers to on the network. It starts as "pisquared", and
//! typing edits it. The rules are the backend's, and the page continues only once the name is valid.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};

use crate::backend::network;

const DEFAULT_NAME: &str = "pisquared";

pub struct Model {
    name: String,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            name: DEFAULT_NAME.to_owned(),
        }
    }
}

impl Model {
    /// The hostname, once it is valid.
    pub fn name(&self) -> Option<String> {
        self.error().is_none().then(|| self.name.clone())
    }

    fn error(&self) -> Option<String> {
        network::validate_hostname(&self.name)
            .err()
            .map(|err| err.to_string())
    }
}

/// Applies a key press: typing edits the hostname, and Backspace removes its last character.
pub fn update(model: &mut Model, key: &KeyEvent) {
    match key.code {
        KeyCode::Backspace => {
            model.name.pop();
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => model.name.push(c),
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
    let mut lines = vec![Line::from(vec![
        Span::styled("> Hostname: ", Style::new().bold()),
        Span::raw(model.name.clone()),
    ])];
    if let Some(error) = model.error() {
        lines.push(Line::styled(format!("    {error}"), Style::new().red()));
    }
    lines.push(Line::default());
    lines.push(Line::styled(
        "Letters, digits, and hyphens only, up to 63 characters, and not starting or ending with a hyphen.",
        Style::new().dim(),
    ));
    lines
}

fn footer(model: &Model) -> String {
    let status = if model.name().is_some() {
        "Enter continues to confirm your choices."
    } else {
        "Fix the hostname to continue."
    };
    format!(
        "{status}\nType to edit, Backspace deletes. Enter continues. Left or Esc goes back. Ctrl-C quits."
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

    fn screen(model: &Model) -> String {
        let mut terminal = Terminal::new(TestBackend::new(120, 12)).unwrap();
        terminal
            .draw(|frame| view(model, frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..12)
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
        let model = Model::default();

        assert_eq!(model.name(), Some("pisquared".to_owned()));
        assert!(screen(&model).contains("Hostname: pisquared"));
    }

    #[test]
    fn typing_and_backspace_edit_the_hostname() {
        let mut model = Model::default();
        for _ in 0..9 {
            update(&mut model, &key(KeyCode::Backspace));
        }
        for c in "mypi".chars() {
            update(&mut model, &key(KeyCode::Char(c)));
        }

        assert_eq!(model.name(), Some("mypi".to_owned()));
    }

    #[test]
    fn an_empty_or_invalid_hostname_is_not_given_and_shows_its_error() {
        let mut model = Model::default();
        for _ in 0..9 {
            update(&mut model, &key(KeyCode::Backspace));
        }
        assert_eq!(model.name(), None);
        assert!(screen(&model).contains("the hostname is empty"));

        update(&mut model, &key(KeyCode::Char('-')));
        assert_eq!(model.name(), None);
        assert!(screen(&model).contains("cannot start or end with a hyphen"));
    }

    #[test]
    fn control_characters_are_not_typed_into_the_hostname() {
        let mut model = Model::default();
        update(
            &mut model,
            &KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
        );

        assert_eq!(model.name(), Some("pisquared".to_owned()));
    }
}
