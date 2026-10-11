mod app;
mod catalogue;
#[cfg(test)]
mod check;
mod decompress;
mod fetch;
mod log;
mod run;
mod select;
mod signature;
#[cfg(test)]
mod test_publisher;
#[cfg(test)]
mod test_server;
#[cfg(test)]
mod tui;

use std::io;
use std::time::Duration;

use crossterm::event;

/// How long the interface waits for a key before it redraws, so progress keeps moving while nothing is typed.
const REDRAW_EVERY: Duration = Duration::from_millis(100);

fn main() -> io::Result<()> {
    let output_dir = std::env::current_dir()?;
    let mut terminal = ratatui::init();
    let result = interact(&mut terminal, app::Model::new(output_dir));
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

#[cfg(test)]
mod tests {
    use async_compression::futures::bufread::XzDecoder;
    use async_compression::futures::write::XzEncoder;
    use futures::executor::block_on;
    use futures::io::{AsyncReadExt, AsyncWriteExt, BufReader};

    #[test]
    fn xz_round_trip_uses_the_decoder() {
        block_on(async {
            let data: Vec<u8> = (0..1_000_000u32).map(|i| (i % 251) as u8).collect();

            let mut encoder = XzEncoder::new(Vec::new());
            encoder.write_all(&data).await.unwrap();
            encoder.close().await.unwrap();
            let compressed = encoder.into_inner();

            let mut decoder = XzDecoder::new(BufReader::new(&compressed[..]));
            let mut out = Vec::new();
            decoder.read_to_end(&mut out).await.unwrap();

            assert_eq!(out, data);
        });
    }
}
