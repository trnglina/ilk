use std::{borrow::Cow, iter::FusedIterator};

use thiserror::Error;

use crate::{
    parser::{
        ident::scan_ident,
        operator::{OperatorClass, OperatorConfig},
    },
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

struct Operand {
    precedence: u16,
    is_number: bool,
}

struct PendingOperator<'a> {
    atom: ParseAtom<'a>,
    precedence: u16,
    class: OperatorClass,
    offset: usize,
    end: usize,
}

enum FrameKind<'a> {
    Root {
        start: usize,
    },
    Group,
    Arguments {
        functor: ParseAtom<'a>,
        argument_count: usize,
    },
}

struct ExpressionFrame<'a> {
    kind: FrameKind<'a>,
    operands: Vec<Operand>,
    operators: Vec<PendingOperator<'a>>,
}

impl<'a> ExpressionFrame<'a> {
    fn new(kind: FrameKind<'a>) -> Self {
        Self {
            kind,
            operands: Vec::new(),
            operators: Vec::new(),
        }
    }
}

enum TermParserState {
    AwaitingFact,
    Operand,
    Operator,
    Done,
    Failed,
}

pub struct TermParser<'a> {
    source: &'a str,
    terminator: u8,
    operators: Option<&'a OperatorConfig<'a>>,
    offset: usize,
    frame_stack: Vec<ExpressionFrame<'a>>,
    state: TermParserState,
}

impl<'a> TermParser<'a> {
    pub fn new(source: &'a str, terminator: u8, operators: Option<&'a OperatorConfig<'a>>) -> Self {
        assert!(terminator.is_ascii(), "terminator must be ascii");

        Self {
            source,
            terminator,
            operators,
            offset: 0,
            state: TermParserState::AwaitingFact,
            frame_stack: Vec::new(),
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

    fn find_operator(&self, prefix: bool) -> Option<PendingOperator<'a>> {
        scan_operator(self.source, self.offset, self.operators?, prefix)
    }

    fn find_operator_end(&self, offset: usize) -> bool {
        scan_operator_end(self.source, offset, self.terminator, self.operators)
    }

    fn last_frame(&mut self) -> &ExpressionFrame<'a> {
        self.frame_stack.last().expect("non-empty expression stack")
    }

    fn last_frame_mut(&mut self) -> &mut ExpressionFrame<'a> {
        self.frame_stack
            .last_mut()
            .expect("non-empty expression stack")
    }

    fn pop_frame(&mut self) -> ExpressionFrame<'a> {
        self.frame_stack.pop().expect("non-empty expression stack")
    }

    fn step(&mut self) -> Result<Option<ParseTerm<'a>>, TermError> {
        match self.state {
            TermParserState::AwaitingFact => {
                self.skip_trivia()?;
                self.begin_parsing_operand()
            }
            TermParserState::Operand => {
                self.skip_trivia()?;
                self.step_operand()
            }
            TermParserState::Operator => {
                self.skip_trivia()?;
                self.step_operator()
            }
            TermParserState::Done | TermParserState::Failed => unreachable!(),
        }
    }

    fn parse_number(&mut self) -> Result<ParseNumber, TermError> {
        let (number, end) = scan_number(self.source, self.offset)?;
        self.offset = end;
        Ok(number)
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
        match scan_trivia(self.source, self.offset) {
            Ok(end) => {
                self.offset = end;
                Ok(())
            }
            Err((error, offset)) => {
                self.offset = offset;
                Err(error)
            }
        }
    }

    fn begin_parsing_operand(&mut self) -> Result<Option<ParseTerm<'a>>, TermError> {
        self.frame_stack
            .push(ExpressionFrame::new(FrameKind::Root { start: self.offset }));
        self.state = TermParserState::Operand;
        Ok(None)
    }

    fn finish_parsing_operand(&mut self, is_number: bool) {
        self.last_frame_mut().operands.push(Operand {
            precedence: 0,
            is_number,
        });
        self.state = TermParserState::Operator;
    }

    fn step_operand(&mut self) -> Result<Option<ParseTerm<'a>>, TermError> {
        let plain_root = self.operators.is_none() && self.frame_stack.len() == 1;
        if plain_root && self.peek().is_some_and(|b| b.is_ascii_digit()) {
            return Err(TermError::UnexpectedCharacter);
        }

        if !plain_root
            && self.peek() == Some(b'-')
            && self.peekn(1).is_some_and(|b| b.is_ascii_digit())
        {
            let number = self.parse_number()?;
            self.finish_parsing_operand(true);
            return Ok(Some(ParseTerm::Number(number)));
        }

        if scan_compound_start(self.source, self.offset).is_none()
            && let Some(operator) = self.find_operator(true)
            && !self.find_operator_end(operator.end)
        {
            self.offset = operator.end;
            self.last_frame_mut().operators.push(operator);
            return Ok(None);
        }

        match self.peek() {
            Some(b'(') if self.operators.is_some() => {
                self.offset += 1;
                self.frame_stack
                    .push(ExpressionFrame::new(FrameKind::Group));
            }
            Some(b'0'..=b'9') => {
                let number = self.parse_number()?;
                self.finish_parsing_operand(true);
                return Ok(Some(ParseTerm::Number(number)));
            }
            Some(_) => {
                let atom = self.parse_atom()?;
                if self.peek() == Some(b'(') {
                    self.offset += 1;
                    self.frame_stack
                        .push(ExpressionFrame::new(FrameKind::Arguments {
                            functor: atom,
                            argument_count: 0,
                        }));
                } else {
                    self.finish_parsing_operand(false);
                    return Ok(Some(ParseTerm::Atom(atom)));
                }
            }
            None => return Err(TermError::UnexpectedEndOfFile),
        }
        Ok(None)
    }

    fn step_operator(&mut self) -> Result<Option<ParseTerm<'a>>, TermError> {
        if let Some(operator) = self.find_operator(false) {
            let frame = self.last_frame();
            if frame.operators.last().is_some_and(|previous| {
                previous.precedence < operator.precedence
                    || (previous.precedence == operator.precedence
                        && operator.class == OperatorClass::Yfx)
            }) {
                return self.reduce_operator().map(Some);
            }

            if frame.operators.last().is_some_and(|previous| {
                previous.precedence == operator.precedence
                    && previous.class == OperatorClass::Xfx
                    && operator.class == OperatorClass::Xfx
            }) {
                self.offset = operator.offset;
                return Err(TermError::UnexpectedCharacter);
            }

            self.offset = operator.end;
            self.last_frame_mut().operators.push(operator);
            self.state = TermParserState::Operand;

            return Ok(None);
        }

        if !self.last_frame().operators.is_empty() {
            return self.reduce_operator().map(Some);
        }

        let operand = self.last_frame_mut().operands.pop().expect("an operand");
        match &mut self.last_frame_mut().kind {
            FrameKind::Root { start } => {
                if operand.is_number {
                    self.offset = *start;
                    return Err(TermError::UnexpectedCharacter);
                }

                match self.peek() {
                    Some(byte) if byte == self.terminator => self.state = TermParserState::Done,
                    Some(b';') => self.state = TermParserState::AwaitingFact,
                    Some(_) => return Err(TermError::UnexpectedCharacter),
                    None => return Err(TermError::UnexpectedEndOfFile),
                }

                self.offset += 1;
                self.frame_stack.clear();

                Ok(Some(ParseTerm::FactEnd))
            }
            FrameKind::Group => {
                match self.peek() {
                    Some(b')') => self.offset += 1,
                    Some(_) => return Err(TermError::UnexpectedCharacter),
                    None => return Err(TermError::UnexpectedEndOfFile),
                }

                self.frame_stack.pop();
                self.last_frame_mut().operands.push(Operand {
                    precedence: 0,
                    ..operand
                });

                Ok(None)
            }
            FrameKind::Arguments { .. } => match self.peek() {
                Some(b',') => {
                    self.offset += 1;

                    if let FrameKind::Arguments { argument_count, .. } =
                        &mut self.last_frame_mut().kind
                    {
                        *argument_count += 1;
                    }

                    self.state = TermParserState::Operand;
                    Ok(None)
                }
                Some(b')') => {
                    self.offset += 1;

                    let frame = self.pop_frame();
                    let FrameKind::Arguments {
                        functor,
                        argument_count,
                    } = frame.kind
                    else {
                        unreachable!()
                    };

                    self.finish_parsing_operand(false);
                    Ok(Some(ParseTerm::Compound(functor, argument_count + 1)))
                }
                Some(_) => Err(TermError::UnexpectedCharacter),
                None => Err(TermError::UnexpectedEndOfFile),
            },
        }
    }

    fn reduce_operator(&mut self) -> Result<ParseTerm<'a>, TermError> {
        let frame = self.last_frame_mut();
        let operator = frame.operators.pop().expect("an operator");
        let right = frame.operands.pop().expect("right operand");
        let right_limit = match operator.class {
            OperatorClass::Xfx | OperatorClass::Yfx => operator.precedence - 1,
            OperatorClass::Xfy | OperatorClass::Fy => operator.precedence,
        };
        if right.precedence > right_limit {
            self.offset = operator.offset;
            return Err(TermError::UnexpectedCharacter);
        }

        let arity = if operator.class.is_prefix() { 1 } else { 2 };
        if !operator.class.is_prefix() {
            let left = frame.operands.pop().expect("left operand");
            let left_limit = if operator.class == OperatorClass::Yfx {
                operator.precedence
            } else {
                operator.precedence - 1
            };
            if left.precedence > left_limit {
                self.offset = operator.offset;
                return Err(TermError::UnexpectedCharacter);
            }
        }
        frame.operands.push(Operand {
            precedence: operator.precedence,
            is_number: false,
        });
        Ok(ParseTerm::Compound(operator.atom, arity))
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

fn scan_number(source: &str, mut offset: usize) -> Result<(ParseNumber, usize), TermError> {
    let bytes = source.as_bytes();
    let start = offset;
    if bytes.get(offset).copied() == Some(b'-') {
        offset += 1;
    }

    let integer_start = offset;
    while bytes.get(offset).is_some_and(u8::is_ascii_digit) {
        offset += 1;
    }
    if offset == integer_start {
        return Err(TermError::UnexpectedCharacter);
    }

    let mut is_real = false;
    if bytes.get(offset).copied() == Some(b'.')
        && bytes.get(offset + 1).is_some_and(u8::is_ascii_digit)
    {
        is_real = true;
        offset += 1;
        while bytes.get(offset).is_some_and(u8::is_ascii_digit) {
            offset += 1;
        }
    }

    if matches!(bytes.get(offset).copied(), Some(b'e' | b'E')) {
        is_real = true;
        offset += 1;
        if matches!(bytes.get(offset).copied(), Some(b'+' | b'-')) {
            offset += 1;
        }
        let exponent_start = offset;
        while bytes.get(offset).is_some_and(u8::is_ascii_digit) {
            offset += 1;
        }
        if offset == exponent_start {
            return Err(TermError::InvalidReal);
        }
    }

    let body = &source[start..offset];
    if is_real {
        if let Ok(value) = body.parse::<f32>()
            && value.is_finite()
        {
            Ok((ParseNumber::Real(RealTerm::new(value)), offset))
        } else {
            Err(TermError::InvalidReal)
        }
    } else {
        if let Ok(value) = body.parse::<i32>() {
            Ok((ParseNumber::Integer(IntegerTerm::new(value)), offset))
        } else {
            Err(TermError::InvalidInteger)
        }
    }
}

fn scan_trivia(source: &str, mut offset: usize) -> Result<usize, (TermError, usize)> {
    loop {
        while source
            .as_bytes()
            .get(offset)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            offset += 1;
        }
        if !source[offset..].starts_with("/*") {
            return Ok(offset);
        }

        let Some(end) = source[offset + 2..].find("*/") else {
            return Err((TermError::UnterminatedComment, offset));
        };
        offset += end + 4;
    }
}

fn scan_compound_start(source: &str, offset: usize) -> Option<usize> {
    let end = scan_ident(source, offset)?;
    (source.as_bytes().get(end) == Some(&b'(')).then_some(end + 1)
}

fn scan_operator<'a>(
    source: &'a str,
    offset: usize,
    operators: &OperatorConfig<'a>,
    prefix: bool,
) -> Option<PendingOperator<'a>> {
    let end = scan_ident(source, offset)?;
    let name = &source[offset..end];
    let operator = operators.get(name, prefix)?;
    Some(PendingOperator {
        atom: ParseAtom {
            body: name,
            quoted: false,
        },
        precedence: operator.precedence,
        class: operator.class,
        offset,
        end,
    })
}

fn scan_operator_end<'a>(
    source: &'a str,
    offset: usize,
    terminator: u8,
    operators: Option<&OperatorConfig<'a>>,
) -> bool {
    let Ok(offset) = scan_trivia(source, offset) else {
        return false;
    };

    match source.as_bytes().get(offset).copied() {
        None | Some(b')' | b',' | b';') => true,
        Some(byte) => {
            byte == terminator
                || (scan_compound_start(source, offset).is_none()
                    && operators.is_some_and(|operators| {
                        scan_operator(source, offset, operators, false).is_some()
                            && scan_operator(source, offset, operators, true).is_none()
                    }))
        }
    }
}
