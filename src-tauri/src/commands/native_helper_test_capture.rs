// Historical framing fixture only; production raw capture uses the actual process pump.
fn drain_native_output(mut stream: impl Read, limit: usize) -> (Vec<u8>, bool) {
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    let mut truncated = false;
    loop {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(size) => {
                let keep = size.min(limit.saturating_sub(output.len()));
                output.extend_from_slice(&buffer[..keep]);
                truncated |= keep < size;
            }
        }
    }
    (output, truncated)
}
