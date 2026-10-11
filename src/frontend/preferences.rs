//! The preferences screen: three menus, for the orchestrator, the harnesses, and the sandboxes.
//!
//! Tab moves between the menus. Up and down move the cursor within the focused menu, Space picks the option under it,
//! and Enter continues once every menu has a choice. The orchestrator takes one choice; a harness or a sandbox may take
//! several. The options and the summary are the backend's; this view holds only the cursor and what is picked.

use crossterm::event::{Event, KeyCode, KeyEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, List, ListState, Paragraph, Wrap};

use crate::backend::preferences::{Choice, Harness, Orchestrator, Preferences, Sandbox};

pub struct Model {
    focus: Focus,
    orchestrator: Menu<Orchestrator>,
    harnesses: Menu<Harness>,
    sandboxes: Menu<Sandbox>,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            focus: Focus::Orchestrator,
            orchestrator: Menu::new(false),
            harnesses: Menu::new(true),
            sandboxes: Menu::new(true),
        }
    }
}

impl Model {
    /// The preferences the operator has made, once every menu has a choice.
    pub fn confirmed(&self) -> Option<Preferences> {
        let orchestrator = self.orchestrator.chosen().first().copied()?;
        let harnesses = self.harnesses.chosen();
        let sandboxes = self.sandboxes.chosen();
        if harnesses.is_empty() || sandboxes.is_empty() {
            return None;
        }
        Some(Preferences {
            orchestrator,
            harnesses,
            sandboxes,
        })
    }

    /// The menus still without a choice, by name.
    fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.orchestrator.chosen().is_empty() {
            missing.push("orchestrator");
        }
        if self.harnesses.chosen().is_empty() {
            missing.push("harness");
        }
        if self.sandboxes.chosen().is_empty() {
            missing.push("sandbox");
        }
        missing
    }

    fn focused(&mut self) -> &mut dyn Pick {
        match self.focus {
            Focus::Orchestrator => &mut self.orchestrator,
            Focus::Harnesses => &mut self.harnesses,
            Focus::Sandboxes => &mut self.sandboxes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Orchestrator,
    Harnesses,
    Sandboxes,
}

impl Focus {
    fn next(self) -> Self {
        match self {
            Self::Orchestrator => Self::Harnesses,
            Self::Harnesses => Self::Sandboxes,
            Self::Sandboxes => Self::Orchestrator,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Orchestrator => Self::Sandboxes,
            Self::Harnesses => Self::Orchestrator,
            Self::Sandboxes => Self::Harnesses,
        }
    }
}

/// A menu's cursor and picks. A single-choice menu keeps only the latest pick.
struct Menu<T: Choice> {
    multiple: bool,
    cursor: usize,
    chosen: Vec<T>,
}

impl<T: Choice> Menu<T> {
    fn new(multiple: bool) -> Self {
        Self {
            multiple,
            cursor: 0,
            chosen: Vec::new(),
        }
    }

    /// The picks, in the order the menu lists them.
    fn chosen(&self) -> Vec<T> {
        T::ALL
            .iter()
            .copied()
            .filter(|choice| self.chosen.contains(choice))
            .collect()
    }
}

/// What the focused menu does with a key. Lets the view work on any menu without knowing its option type.
trait Pick {
    fn move_cursor(&mut self, by: isize);

    fn toggle(&mut self);
}

impl<T: Choice> Pick for Menu<T> {
    fn move_cursor(&mut self, by: isize) {
        let last = (T::ALL.len() - 1) as isize;
        self.cursor = (self.cursor as isize + by).clamp(0, last) as usize;
    }

    fn toggle(&mut self) {
        let choice = T::ALL[self.cursor];
        if let Some(index) = self.chosen.iter().position(|pick| *pick == choice) {
            self.chosen.remove(index);
        } else {
            if !self.multiple {
                self.chosen.clear();
            }
            self.chosen.push(choice);
        }
    }
}

/// Applies a terminal event: Tab moves the focus, and the arrow keys and Space act on the focused menu.
pub fn update(model: &mut Model, event: &Event) {
    let Event::Key(key) = event else {
        return;
    };
    if key.kind != KeyEventKind::Press {
        return;
    }
    match key.code {
        KeyCode::Tab => model.focus = model.focus.next(),
        KeyCode::BackTab => model.focus = model.focus.previous(),
        KeyCode::Up => model.focused().move_cursor(-1),
        KeyCode::Down => model.focused().move_cursor(1),
        KeyCode::Char(' ') => model.focused().toggle(),
        _ => {}
    }
}

pub fn view(model: &Model, frame: &mut Frame) {
    let [menus_area, footer_area] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(3)]).areas(frame.area());
    let [orchestrator_area, harnesses_area, sandboxes_area] =
        Layout::horizontal([Constraint::Fill(1); 3]).areas(menus_area);

    render_menu(
        frame,
        orchestrator_area,
        &model.orchestrator,
        "Orchestrator (choose one)",
        model.focus == Focus::Orchestrator,
    );
    render_menu(
        frame,
        harnesses_area,
        &model.harnesses,
        "Harnesses (choose one or more)",
        model.focus == Focus::Harnesses,
    );
    render_menu(
        frame,
        sandboxes_area,
        &model.sandboxes,
        "Sandboxes (choose one or more)",
        model.focus == Focus::Sandboxes,
    );

    frame.render_widget(
        Paragraph::new(footer(model)).wrap(Wrap { trim: true }),
        footer_area,
    );
}

fn render_menu<T: Choice>(
    frame: &mut Frame,
    area: Rect,
    menu: &Menu<T>,
    title: &str,
    focused: bool,
) {
    let items: Vec<String> = T::ALL
        .iter()
        .map(|choice| {
            let mark = if menu.chosen.contains(choice) {
                "[x]"
            } else {
                "[ ]"
            };
            format!("{mark} {}", choice.name())
        })
        .collect();
    let border = if focused {
        Style::new().bold()
    } else {
        Style::new()
    };
    let mut list = List::new(items).block(Block::bordered().title(title).border_style(border));
    if focused {
        list = list
            .highlight_style(Style::new().reversed())
            .highlight_symbol("> ");
    }
    let mut state = ListState::default();
    state.select(Some(menu.cursor));
    frame.render_stateful_widget(list, area, &mut state);
}

fn footer(model: &Model) -> String {
    let status = match model.missing().as_slice() {
        [] => "Enter continues to confirm your choices.".to_owned(),
        missing => format!("Still to choose: {}.", missing.join(", ")),
    };
    format!(
        "{status}\nTab moves between menus. Up and down move, Space picks, Enter continues. Esc quits."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const WIDTH: u16 = 120;
    const HEIGHT: u16 = 12;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
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

    /// Picks the option under the cursor of the focused menu.
    fn pick(model: &mut Model) {
        update(model, &key(KeyCode::Char(' ')));
    }

    #[test]
    fn nothing_is_confirmed_until_every_menu_has_a_choice() {
        let mut model = Model::default();
        assert_eq!(model.missing(), ["orchestrator", "harness", "sandbox"]);

        pick(&mut model);
        update(&mut model, &key(KeyCode::Tab));
        pick(&mut model);
        assert!(model.confirmed().is_none());
        assert_eq!(model.missing(), ["sandbox"]);

        update(&mut model, &key(KeyCode::Tab));
        pick(&mut model);
        assert_eq!(
            model.confirmed(),
            Some(Preferences {
                orchestrator: Orchestrator::Herdr,
                harnesses: vec![Harness::Pi],
                sandboxes: vec![Sandbox::Nono],
            })
        );
    }

    #[test]
    fn a_harness_menu_takes_several_choices_and_space_removes_one() {
        let mut model = Model::default();
        update(&mut model, &key(KeyCode::Tab));
        pick(&mut model);
        update(&mut model, &key(KeyCode::Down));
        pick(&mut model);
        update(&mut model, &key(KeyCode::Down));
        pick(&mut model);
        assert_eq!(
            model.harnesses.chosen(),
            [Harness::Pi, Harness::OpenCode, Harness::ClaudeCode]
        );

        update(&mut model, &key(KeyCode::Up));
        update(&mut model, &key(KeyCode::Up));
        pick(&mut model);
        assert_eq!(
            model.harnesses.chosen(),
            [Harness::OpenCode, Harness::ClaudeCode]
        );
    }

    #[test]
    fn a_single_choice_menu_keeps_only_the_latest_pick() {
        let mut menu = Menu::<Harness>::new(false);
        menu.toggle();
        menu.move_cursor(1);
        menu.toggle();

        assert_eq!(menu.chosen(), [Harness::OpenCode]);
    }

    #[test]
    fn the_cursor_stops_at_the_ends_of_its_menu() {
        let mut model = Model::default();
        update(&mut model, &key(KeyCode::Tab));
        for _ in 0..10 {
            update(&mut model, &key(KeyCode::Down));
        }
        pick(&mut model);
        assert_eq!(model.harnesses.chosen(), [Harness::ClaudeCode]);

        update(&mut model, &key(KeyCode::BackTab));
        update(&mut model, &key(KeyCode::Up));
        assert_eq!(model.orchestrator.cursor, 0);
    }

    #[test]
    fn the_rendered_screen_marks_each_pick_and_names_what_is_missing() {
        let mut model = Model::default();
        pick(&mut model);
        update(&mut model, &key(KeyCode::Tab));
        update(&mut model, &key(KeyCode::Down));
        pick(&mut model);

        let rendered = words(&screen(&model));

        assert!(rendered.contains("[x] herdr"), "{rendered}");
        assert!(rendered.contains("[x] opencode"), "{rendered}");
        assert!(rendered.contains("[ ] pi agent"), "{rendered}");
        assert!(rendered.contains("[ ] nono"), "{rendered}");
        assert!(rendered.contains("Still to choose: sandbox."), "{rendered}");
    }
}
