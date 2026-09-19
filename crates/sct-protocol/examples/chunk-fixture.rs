fn main() {
    let mut state = 0x123456789abcdef0u64;
    let bytes: Vec<u8> = (0..12 * 1024 * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect();
    let chunks: Vec<_> = sct_protocol::chunker(std::io::Cursor::new(bytes))
        .map(|v| {
            let c = v.unwrap();
            (c.length, sct_protocol::digest(&c.data))
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&chunks).unwrap());
}
