use std::{borrow::Cow, iter::FusedIterator, num::NonZeroUsize};

use thiserror::Error;

use crate::{
    parser::ident::scan_ident,
    term::{IntegerTerm, IntoTerm, RealTerm, Term, TermTable},
};

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum TermError {
    #[error("invalid escape sequence")]
    InvalidEscape,
    #[error("integer out of range")]
    InvalidInteger,
    #[error("invalid real number")]
    InvalidReal,
    #[error("unexpected character")]
    UnexpectedCharacter,
    #[error("unexpected end of file")]
    UnexpectedEndOfFile,
    #[error("unterminated block comment")]
    UnterminatedComment,
    #[error("unterminated quoted atom")]
    UnterminatedQuotedAtom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseAtom<'a> {
    body: &'a str,
    quoted: bool,
}

impl IntoTerm for ParseAtom<'_> {
    fn into_term(self, terms: &mut TermTable) -> Term {
        Term::Atom(terms.atom(&self.value()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseNumber {
    Integer(IntegerTerm),
    Real(RealTerm),
}

impl IntoTerm for ParseNumber {
    fn into_term(self, _terms: &mut TermTable) -> Term {
        match self {
            Self::Integer(value) => Term::Integer(value),
            Self::Real(value) => Term::Real(value),
        }
    }
}

impl<'a> ParseAtom<'a> {
    fn value(self) -> Cow<'a, str> {
        if !self.quoted || !self.body.contains(['\\', '\'']) {
            return Cow::Borrowed(self.body);
        }

        let mut value = String::with_capacity(self.body.len());
        let mut position = 0;
        while position < self.body.len() {
            let current = self.body[position..].chars().next().unwrap();
            position += current.len_utf8();
            match current {
                '\'' => {
                    position += 1;
                    value.push('\'');
                }
                '\\' => {
                    let escape = self.body[position..].chars().next().unwrap();
                    position += escape.len_utf8();
                    match escape {
                        '\\' => value.push('\\'),
                        '\'' => value.push('\''),
                        'n' => value.push('\n'),
                        'r' => value.push('\r'),
                        't' => value.push('\t'),
                        'u' | 'U' => {
                            let digits = if escape == 'u' { 4 } else { 8 };
                            let end = position + digits;
                            let scalar = u32::from_str_radix(&self.body[position..end], 16)
                                .expect("valid unicode escape");
                            value.push(char::from_u32(scalar).expect("valid unicode scalar"));
                            position = end;
                        }
                        _ => unreachable!("valid escape"),
                    }
                }
                _ => value.push(current),
            }
        }
        Cow::Owned(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseTerm<'a> {
    Atom(ParseAtom<'a>),
    Number(ParseNumber),
    Compound(ParseAtom<'a>, usize),
    FactEnd,
}

struct CompoundFrame<'a> {
    functor: ParseAtom<'a>,
    args: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TermParserState {
    AwaitingFact,
    AwaitingTerm { depth: NonZeroUsize },
    FinishedTerm { depth: usize },
    Done,
    Failed,
}

pub struct TermParser<'a> {
    source: &'a str,
    terminator: u8,
    offset: usize,
    state: TermParserState,
    compounds: Vec<CompoundFrame<'a>>,
}

impl<'a> TermParser<'a> {
    pub fn new(source: &'a str, terminator: u8) -> Self {
        assert!(terminator.is_ascii(), "terminator must be ascii");
        Self {
            source,
            terminator,
            offset: 0,
            state: TermParserState::AwaitingFact,
            compounds: Vec::new(),
        }
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    fn peek(&self) -> Option<u8> {
        self.source.as_bytes().get(self.offset).copied()
    }

    fn peekn(&self, off: usize) -> Option<u8> {
        self.source.as_bytes().get(self.offset + off).copied()
    }

    fn step(&mut self) -> Result<Option<ParseTerm<'a>>, TermError> {
        match self.state {
            TermParserState::AwaitingFact => {
                self.skip_trivia()?;
                return self.parse_term_head(0);
            }
            TermParserState::AwaitingTerm { depth } => {
                self.skip_trivia()?;
                return self.parse_term(depth);
            }
            TermParserState::FinishedTerm { depth } => {
                self.skip_trivia()?;
                if let Some(depth) = NonZeroUsize::new(depth) {
                    match self.peek() {
                        Some(b',') => {
                            self.offset += 1;
                            self.compounds.last_mut().expect("open compound").args += 1;
                            self.state = TermParserState::AwaitingTerm { depth };
                            return Ok(None);
                        }
                        Some(b')') => {
                            self.offset += 1;
                            self.state = TermParserState::FinishedTerm {
                                depth: depth.get() - 1,
                            };
                            let frame = self.compounds.pop().expect("open compound");
                            return Ok(Some(ParseTerm::Compound(frame.functor, frame.args + 1)));
                        }
                        Some(_) => return Err(TermError::UnexpectedCharacter),
                        None => return Err(TermError::UnexpectedEndOfFile),
                    }
                } else {
                    match self.peek() {
                        Some(byte) if byte == self.terminator => {
                            self.offset += 1;
                            self.state = TermParserState::Done;
                            return Ok(Some(ParseTerm::FactEnd));
                        }
                        Some(b';') => {
                            self.offset += 1;
                            self.state = TermParserState::AwaitingFact;
                            return Ok(Some(ParseTerm::FactEnd));
                        }
                        Some(_) => return Err(TermError::UnexpectedCharacter),
                        None => return Err(TermError::UnexpectedEndOfFile),
                    }
                }
            }
            TermParserState::Done | TermParserState::Failed => unreachable!(),
        }
    }

    fn parse_term(&mut self, depth: NonZeroUsize) -> Result<Option<ParseTerm<'a>>, TermError> {
        match self.peek() {
            Some(byte)
                if byte.is_ascii_digit()
                    || (byte == b'-' && self.peekn(1).is_some_and(|c| c.is_ascii_digit())) =>
            {
                let number = self.parse_number()?;
                self.state = TermParserState::FinishedTerm { depth: depth.get() };
                Ok(Some(ParseTerm::Number(number)))
            }
            Some(_) => self.parse_term_head(depth.get()),
            None => Err(TermError::UnexpectedEndOfFile),
        }
    }

    fn parse_term_head(&mut self, depth: usize) -> Result<Option<ParseTerm<'a>>, TermError> {
        let atom = self.parse_atom()?;
        if self.peek() == Some(b'(') {
            self.offset += 1;
            let depth = NonZeroUsize::new(depth + 1).expect("compound depth exceeds usize::MAX");
            self.compounds.push(CompoundFrame {
                functor: atom,
                args: 0,
            });
            self.state = TermParserState::AwaitingTerm { depth };
            Ok(None)
        } else {
            self.state = TermParserState::FinishedTerm { depth };
            Ok(Some(ParseTerm::Atom(atom)))
        }
    }

    fn parse_number(&mut self) -> Result<ParseNumber, TermError> {
        let start = self.offset;
        if self.peek() == Some(b'-') {
            self.offset += 1;
        }

        let integer_start = self.offset;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.offset += 1;
        }
        if self.offset == integer_start {
            self.offset = start;
            return Err(TermError::UnexpectedCharacter);
        }

        let mut is_real = false;
        if self.peek() == Some(b'.') && self.peekn(1).is_some_and(|c| c.is_ascii_digit()) {
            is_real = true;
            self.offset += 1;
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.offset += 1;
            }
        }

        if matches!(self.peek(), Some(b'e' | b'E')) {
            is_real = true;
            self.offset += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.offset += 1;
            }
            let exponent_start = self.offset;
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.offset += 1;
            }
            if self.offset == exponent_start {
                self.offset = start;
                return Err(TermError::InvalidReal);
            }
        }

        let body = &self.source[start..self.offset];
        if is_real {
            if let Ok(value) = body.parse::<f32>()
                && value.is_finite()
            {
                Ok(ParseNumber::Real(RealTerm::new(value)))
            } else {
                self.offset = start;
                Err(TermError::InvalidReal)
            }
        } else {
            if let Ok(value) = body.parse::<i32>() {
                Ok(ParseNumber::Integer(IntegerTerm::new(value)))
            } else {
                self.offset = start;
                Err(TermError::InvalidInteger)
            }
        }
    }

    fn parse_atom(&mut self) -> Result<ParseAtom<'a>, TermError> {
        if let Some(end) = scan_ident(self.source, self.offset) {
            let start = self.offset;
            self.offset = end;
            return Ok(ParseAtom {
                body: &self.source[start..end],
                quoted: false,
            });
        }

        match self.peek() {
            Some(b'\'') => self.parse_quoted_atom(),
            Some(_) => Err(TermError::UnexpectedCharacter),
            None => Err(TermError::UnexpectedEndOfFile),
        }
    }

    fn parse_quoted_atom(&mut self) -> Result<ParseAtom<'a>, TermError> {
        self.offset += 1;
        let content_start = self.offset;

        while self.offset < self.source.len() {
            let current = self.source[self.offset..].chars().next().unwrap();
            if current == '\'' {
                if self.peekn(1) == Some(b'\'') {
                    self.offset += 2;
                    continue;
                }
                let content_end = self.offset;
                self.offset += 1;
                return Ok(ParseAtom {
                    body: &self.source[content_start..content_end],
                    quoted: true,
                });
            }

            if current == '\\' {
                self.offset += 1;
                let Some(escape) = self.source[self.offset..].chars().next() else {
                    break;
                };
                self.offset += escape.len_utf8();
                match escape {
                    '\\' | '\'' | 'n' | 'r' | 't' => {}
                    'u' => self.skip_unicode_escape(4)?,
                    'U' => self.skip_unicode_escape(8)?,
                    _ => return Err(TermError::InvalidEscape),
                }
                continue;
            }

            if current.is_control() {
                return Err(TermError::UnexpectedCharacter);
            }
            self.offset += current.len_utf8();
        }

        Err(TermError::UnterminatedQuotedAtom)
    }

    fn skip_unicode_escape(&mut self, digits: usize) -> Result<(), TermError> {
        let digit_end = self.offset.saturating_add(digits).min(self.source.len());
        let parsed = self
            .source
            .get(self.offset..digit_end)
            .filter(|value| {
                value.len() == digits && value.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            .and_then(|value| u32::from_str_radix(value, 16).ok())
            .and_then(char::from_u32);

        if parsed.is_none() {
            return Err(TermError::InvalidEscape);
        }

        self.offset = digit_end;
        Ok(())
    }

    fn skip_trivia(&mut self) -> Result<(), TermError> {
        loop {
            while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
                self.offset += 1;
            }
            if !self.source[self.offset..].starts_with("/*") {
                return Ok(());
            }

            let Some(end) = self.source[self.offset + 2..].find("*/") else {
                return Err(TermError::UnterminatedComment);
            };

            self.offset += end + 4;
        }
    }
}

impl<'a> Iterator for TermParser<'a> {
    type Item = Result<ParseTerm<'a>, TermError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if matches!(self.state, TermParserState::Done | TermParserState::Failed) {
                return None;
            }

            match self.step() {
                Ok(Some(event)) => return Some(Ok(event)),
                Ok(None) => {}
                Err(error) => {
                    self.state = TermParserState::Failed;
                    return Some(Err(error));
                }
            }
        }
    }
}

impl FusedIterator for TermParser<'_> {}
