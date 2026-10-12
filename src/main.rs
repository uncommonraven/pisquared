mod backend;
mod frontend;
#[cfg(test)]
mod testing;

use std::io;
use std::time::Duration;

use crossterm::event;
use frontend::app;

/// How long the interface waits for a key before it redraws, so progress keeps moving while nothing is typed.
const REDRAW_EVERY: Duration = Duration::from_millis(100);

fn main() -> io::Result<()> {
    let output = tempfile::tempdir()?;
    let mut terminal = ratatui::init();
    let result = interact(&mut terminal, app::Model::new(output.path().to_path_buf()));
    ratatui::restore();
    result
}

fn interact(terminal: &mut ratatui::DefaultTerminal, mut model: app::Model) -> io::Result<()> {
    loop {
        app::tick(&mut model);
        terminal.draw(|frame| app::view(&model, frame))?;
        if event::poll(REDRAW_EVERY)? && app::update(&mut model, &event::read()?) {
            return Ok(());
        }
    }
}
