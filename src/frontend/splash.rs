//! The opening screen: the title and version, then a menu of Begin and Quit.
//!
//! Up and down move the cursor between the options and stop at the ends. Enter takes the option under the cursor, and
//! Esc or Ctrl-C quits. The decisions here are plain values; the terminal only renders them.

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

const TITLE: &str = "pisquared";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const OPTIONS: [(Choice, &str); 2] = [(Choice::Begin, "Begin"), (Choice::Quit, "Quit")];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Begin,
    Quit,
}

#[derive(Debug, Default)]
pub struct Model {
    cursor: usize,
}

/// Applies a terminal event and reports the option taken, if any.
pub fn update(model: &mut Model, event: &Event) -> Option<Choice> {
    let Event::Key(key) = event else {
        return None;
    };
    if key.kind != KeyEventKind::Press {
        return None;
    }
    match key.code {
        KeyCode::Up => model.cursor = model.cursor.saturating_sub(1),
        KeyCode::Down => {
            let last = OPTIONS.len() - 1;
            model.cursor = (model.cursor + 1).min(last);
        }
        KeyCode::Enter => return Some(OPTIONS[model.cursor].0),
        KeyCode::Esc => return Some(Choice::Quit),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return Some(Choice::Quit);
        }
        _ => {}
    }
    None
}

pub fn view(model: &Model, frame: &mut Frame, area: Rect) {
    let [_, title_area, version_area, _, menu_area, hint_area, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(OPTIONS.len() as u16),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(area);

    frame.render_widget(
        Paragraph::new(TITLE)
            .style(Style::new().bold())
            .alignment(Alignment::Center),
        title_area,
    );
    frame.render_widget(
        Paragraph::new(format!("v{VERSION}")).alignment(Alignment::Center),
        version_area,
    );

    let lines: Vec<Line> = OPTIONS
        .iter()
        .enumerate()
        .map(|(index, (_, label))| {
            let style = if index == model.cursor {
                Style::new().reversed()
            } else {
                Style::new()
            };
            Line::styled(format!(" {label} "), style)
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines).alignment(Alignment::Center),
        menu_area,
    );

    frame.render_widget(
        Paragraph::new("Up and down move, Enter chooses. Esc quits.").alignment(Alignment::Center),
        hint_area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const WIDTH: u16 = 60;
    const HEIGHT: u16 = 20;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
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

    #[test]
    fn enter_on_begin_begins() {
        let mut model = Model::default();

        assert_eq!(
            update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(Choice::Begin)
        );
    }

    #[test]
    fn down_then_enter_quits_and_the_cursor_stops_at_the_ends() {
        let mut model = Model::default();
        assert_eq!(
            update(&mut model, &key(KeyCode::Up, KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(Choice::Begin)
        );

        for _ in 0..5 {
            update(&mut model, &key(KeyCode::Down, KeyModifiers::NONE));
        }
        assert_eq!(
            update(&mut model, &key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(Choice::Quit)
        );
    }

    #[test]
    fn escape_and_control_c_quit() {
        let mut model = Model::default();

        assert_eq!(
            update(&mut model, &key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(Choice::Quit)
        );
        assert_eq!(
            update(&mut model, &key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(Choice::Quit)
        );
    }

    #[test]
    fn the_screen_shows_the_title_version_and_both_options() {
        let rendered = screen(&Model::default());

        assert!(rendered.contains("pisquared"), "{rendered}");
        assert!(rendered.contains(&format!("v{VERSION}")), "{rendered}");
        assert!(rendered.contains("Begin"), "{rendered}");
        assert!(rendered.contains("Quit"), "{rendered}");
    }
}
