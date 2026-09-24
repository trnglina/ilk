fn is_word_affix(byte: u8) -> bool {
    matches!(byte, b'$' | b'?' | b'#')
}

pub fn is_word_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || is_word_affix(byte)
}

pub fn scan_word_ident(source: &str, mut offset: usize) -> usize {
    let bytes = source.as_bytes();
    if bytes.get(offset).is_some_and(|byte| is_word_affix(*byte)) {
        offset += 1;
    }
    let body_start = offset;
    while bytes.get(offset).is_some_and(u8::is_ascii_alphanumeric) {
        offset += 1;
    }
    while offset > body_start
        && bytes.get(offset) == Some(&b'_')
        && bytes.get(offset + 1).is_some_and(u8::is_ascii_alphanumeric)
    {
        offset += 1;
        while bytes.get(offset).is_some_and(u8::is_ascii_alphanumeric) {
            offset += 1;
        }
    }
    if offset > body_start && bytes.get(offset).is_some_and(|byte| is_word_affix(*byte)) {
        offset += 1;
    }
    offset
}

fn is_symbol(byte: u8) -> bool {
    b"&*+-./:<=>@\\^~".contains(&byte)
}

pub fn scan_ident(source: &str, offset: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    match bytes.get(offset).copied()? {
        byte if is_word_ident_start(byte) => Some(scan_word_ident(source, offset)),
        byte if is_symbol(byte) => {
            let mut end = offset + 1;
            while bytes.get(end).is_some_and(|byte| is_symbol(*byte))
                && !source[end..].starts_with("/*")
            {
                end += 1;
            }
            Some(end)
        }
        _ => None,
    }
}
