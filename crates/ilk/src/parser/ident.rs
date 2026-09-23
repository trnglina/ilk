pub fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'$' | b'?' | b'#')
}

pub fn scan_ident(source: &str, mut offset: usize) -> usize {
    while source
        .as_bytes()
        .get(offset)
        .is_some_and(|byte| is_ident_start(*byte) || byte.is_ascii_digit() || *byte == b'_')
    {
        offset += 1;
    }
    offset
}
