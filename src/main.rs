#[cfg(test)]
mod catalogue;
#[cfg(test)]
mod check;
#[cfg(test)]
mod fetch;
#[cfg(test)]
mod signature;
#[cfg(test)]
mod test_server;
#[cfg(test)]
mod tui;

fn main() {
    println!("pisquared {}", env!("CARGO_PKG_VERSION"));
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
