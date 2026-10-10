use crossterm::event::{Event, KeyCode, KeyEventKind};
use ratatui::Frame;
use ratatui::widgets::Paragraph;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Model {
    pub quit: bool,
}

pub fn update(model: &mut Model, event: &Event) {
    if let Event::Key(key) = event
        && key.kind == KeyEventKind::Press
        && key.code == KeyCode::Char('q')
    {
        model.quit = true;
    }
}

pub fn view(model: &Model, frame: &mut Frame) {
    let text = if model.quit {
        "pisquared: quit"
    } else {
        "pisquared: press q to quit"
    };
    frame.render_widget(Paragraph::new(text), frame.area());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn render(model: &Model) -> String {
        let mut terminal = Terminal::new(TestBackend::new(40, 1)).unwrap();
        terminal.draw(|frame| view(model, frame)).unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn renders_a_frame_to_the_test_backend() {
        let model = Model::default();

        assert!(render(&model).contains("press q to quit"));
    }

    #[test]
    fn q_quits() {
        let mut model = Model::default();

        update(&mut model, &key(KeyCode::Char('q')));

        assert!(model.quit);
        assert!(render(&model).contains("pisquared: quit"));
    }

    #[test]
    fn other_keys_do_not_quit() {
        let mut model = Model::default();

        update(&mut model, &key(KeyCode::Char('x')));
        update(&mut model, &key(KeyCode::Enter));

        assert_eq!(model, Model { quit: false });
    }

    #[test]
    fn key_release_does_not_quit() {
        let mut model = Model::default();
        let mut release = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;

        update(&mut model, &Event::Key(release));

        assert!(!model.quit);
    }
}
