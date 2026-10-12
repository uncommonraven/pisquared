//! The preferences screens: one menu each for the orchestrator, the harnesses, and the sandboxes.
//!
//! Each menu has a screen of its own. Up and down move the cursor, and Space picks the option under it. Enter continues
//! once the screen has a pick, moving on to the next screen, and from the last screen to the confirmation once every
//! menu has a choice. Left goes back to the previous screen. The orchestrator takes one choice; a harness or a sandbox
//! may take several. The options and the summary are the backend's; this view holds only the cursor and what is picked.

use crossterm::event::{Event, KeyCode, KeyEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, List, ListState, Paragraph, Wrap};

use crate::backend::preferences::{Choice, Harness, Orchestrator, Preferences, Sandbox};

pub struct Model {
    screen: Screen,
    orchestrator: Menu<Orchestrator>,
    harnesses: Menu<Harness>,
    sandboxes: Menu<Sandbox>,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            screen: Screen::Orchestrator,
            orchestrator: Menu::new(false),
            harnesses: Menu::new(true),
            sandboxes: Menu::new(false),
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

    /// Continues from the current screen once it has a pick. From the last screen it returns the preferences, if every
    /// menu has a choice.
    pub fn enter(&mut self) -> Option<Preferences> {
        if !self.current().picked() {
            return None;
        }
        match self.screen.next() {
            Some(next) => {
                self.screen = next;
                None
            }
            None => self.confirmed(),
        }
    }

    /// The position of the current screen among the preferences screens, from the first.
    pub fn page(&self) -> usize {
        match self.screen {
            Screen::Orchestrator => 0,
            Screen::Harnesses => 1,
            Screen::Sandboxes => 2,
        }
    }

    /// Goes back to the previous screen. Reports false on the first screen, where going back leaves the preferences.
    pub fn back(&mut self) -> bool {
        match self.screen.previous() {
            Some(previous) => {
                self.screen = previous;
                true
            }
            None => false,
        }
    }

    fn current(&self) -> &dyn Pick {
        match self.screen {
            Screen::Orchestrator => &self.orchestrator,
            Screen::Harnesses => &self.harnesses,
            Screen::Sandboxes => &self.sandboxes,
        }
    }

    fn current_mut(&mut self) -> &mut dyn Pick {
        match self.screen {
            Screen::Orchestrator => &mut self.orchestrator,
            Screen::Harnesses => &mut self.harnesses,
            Screen::Sandboxes => &mut self.sandboxes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Orchestrator,
    Harnesses,
    Sandboxes,
}

impl Screen {
    fn next(self) -> Option<Self> {
        match self {
            Self::Orchestrator => Some(Self::Harnesses),
            Self::Harnesses => Some(Self::Sandboxes),
            Self::Sandboxes => None,
        }
    }

    fn previous(self) -> Option<Self> {
        match self {
            Self::Orchestrator => None,
            Self::Harnesses => Some(Self::Orchestrator),
            Self::Sandboxes => Some(Self::Harnesses),
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Orchestrator => "Orchestrator (choose one)",
            Self::Harnesses => "Harnesses (choose one or more)",
            Self::Sandboxes => "Sandboxes (choose one)",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Orchestrator => "the orchestrator",
            Self::Harnesses => "the harnesses",
            Self::Sandboxes => "the sandboxes",
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

/// What the current menu does with a key. Lets the view work on any menu without knowing its option type.
trait Pick {
    fn move_cursor(&mut self, by: isize);

    fn toggle(&mut self);

    fn picked(&self) -> bool;
}

impl<T: Choice> Pick for Menu<T> {
    fn move_cursor(&mut self, by: isize) {
        let last = (T::ALL.len() - 1) as isize;
        self.cursor = (self.cursor as isize + by).clamp(0, last) as usize;
    }

    fn toggle(&mut self) {
        let choice = T::ALL[self.cursor];
        if self.multiple {
            if let Some(index) = self.chosen.iter().position(|pick| *pick == choice) {
                self.chosen.remove(index);
            } else {
                self.chosen.push(choice);
            }
        } else if !self.chosen.contains(&choice) {
            self.chosen.clear();
            self.chosen.push(choice);
        }
    }

    fn picked(&self) -> bool {
        !self.chosen.is_empty()
    }
}

/// Applies a terminal event: the arrow keys and Space act on the current menu.
pub fn update(model: &mut Model, event: &Event) {
    let Event::Key(key) = event else {
        return;
    };
    if key.kind != KeyEventKind::Press {
        return;
    }
    match key.code {
        KeyCode::Up => model.current_mut().move_cursor(-1),
        KeyCode::Down => model.current_mut().move_cursor(1),
        KeyCode::Char(' ') => model.current_mut().toggle(),
        _ => {}
    }
}

pub fn view(model: &Model, frame: &mut Frame, area: Rect) {
    let [menu_area, footer_area] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(3)]).areas(area);

    match model.screen {
        Screen::Orchestrator => {
            render_menu(frame, menu_area, &model.orchestrator, model.screen.title())
        }
        Screen::Harnesses => render_menu(frame, menu_area, &model.harnesses, model.screen.title()),
        Screen::Sandboxes => render_menu(frame, menu_area, &model.sandboxes, model.screen.title()),
    }

    frame.render_widget(
        Paragraph::new(footer(model)).wrap(Wrap { trim: true }),
        footer_area,
    );
}

fn render_menu<T: Choice>(frame: &mut Frame, area: Rect, menu: &Menu<T>, title: &str) {
    let items: Vec<String> = T::ALL
        .iter()
        .map(|choice| {
            let picked = menu.chosen.contains(choice);
            let mark = match (menu.multiple, picked) {
                (true, true) => "[x]",
                (true, false) => "[ ]",
                (false, true) => "(•)",
                (false, false) => "( )",
            };
            format!("{mark} {}", choice.name())
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::bordered()
                .title(title)
                .border_style(Style::new().bold()),
        )
        .highlight_style(Style::new().reversed())
        .highlight_symbol("> ");
    let mut state = ListState::default();
    state.select(Some(menu.cursor));
    frame.render_stateful_widget(list, area, &mut state);
}

fn footer(model: &Model) -> String {
    let status = if !model.current().picked() {
        "Pick an option to continue.".to_owned()
    } else {
        match model.screen.next() {
            Some(next) => format!("Enter continues to {}.", next.name()),
            None => "Enter continues to confirm your choices.".to_owned(),
        }
    };
    format!("{status}\nUp and down move, Space picks, Enter continues. Left goes back. Esc quits.")
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

    /// Picks the option under the cursor of the current menu.
    fn pick(model: &mut Model) {
        update(model, &key(KeyCode::Char(' ')));
    }

    #[test]
    fn enter_moves_through_the_screens_and_confirms_only_after_the_last() {
        let mut model = Model::default();
        assert_eq!(model.enter(), None, "nothing is picked, so nothing moves");
        assert_eq!(model.screen, Screen::Orchestrator);

        pick(&mut model);
        assert_eq!(model.enter(), None);
        assert_eq!(model.screen, Screen::Harnesses);

        pick(&mut model);
        assert_eq!(model.enter(), None);
        assert_eq!(model.screen, Screen::Sandboxes);

        pick(&mut model);
        assert_eq!(
            model.enter(),
            Some(Preferences {
                orchestrator: Orchestrator::Herdr,
                harnesses: vec![Harness::Pi],
                sandboxes: vec![Sandbox::Nono],
            })
        );
    }

    #[test]
    fn back_returns_to_the_previous_screen_and_keeps_the_picks() {
        let mut model = Model::default();
        pick(&mut model);
        model.enter();

        assert!(model.back());
        assert_eq!(model.screen, Screen::Orchestrator);
        assert_eq!(model.orchestrator.chosen(), [Orchestrator::Herdr]);
        assert!(!model.back(), "the first screen has no previous screen");
    }

    #[test]
    fn a_harness_menu_takes_several_choices_and_space_removes_one() {
        let mut model = Model::default();
        pick(&mut model);
        model.enter();
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
    fn a_single_choice_menu_stays_on_its_pick_when_space_is_pressed_again() {
        let mut model = Model::default();
        pick(&mut model);
        model.enter();
        pick(&mut model);
        model.enter();
        assert_eq!(model.screen, Screen::Sandboxes);

        pick(&mut model);
        pick(&mut model);

        assert_eq!(model.sandboxes.chosen(), [Sandbox::Nono]);
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
        pick(&mut model);
        model.enter();
        for _ in 0..10 {
            update(&mut model, &key(KeyCode::Down));
        }
        pick(&mut model);
        assert_eq!(model.harnesses.chosen(), [Harness::ClaudeCode]);

        model.back();
        update(&mut model, &key(KeyCode::Up));
        assert_eq!(model.orchestrator.cursor, 0);
    }

    #[test]
    fn each_screen_shows_only_its_own_menu() {
        let mut model = Model::default();
        pick(&mut model);

        let rendered = words(&screen(&model));
        assert!(rendered.contains("(•) herdr"), "{rendered}");
        assert!(!rendered.contains("pi agent"), "{rendered}");
        assert!(
            rendered.contains("Enter continues to the harnesses."),
            "{rendered}"
        );

        model.enter();
        let rendered = words(&screen(&model));
        assert!(rendered.contains("[ ] pi agent"), "{rendered}");
        assert!(!rendered.contains("herdr"), "{rendered}");
        assert!(
            rendered.contains("Pick an option to continue."),
            "{rendered}"
        );
    }
}
