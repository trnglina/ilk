pub fn scan_ident(source: &str, mut offset: usize) -> usize {
    while source
        .as_bytes()
        .get(offset)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
    {
        offset += 1;
    }
    offset
}
