use std::{collections::HashMap, iter::FusedIterator, ops::Range};

use thiserror::Error;

use crate::parser::{
    ident::{is_ident_start, scan_ident},
    term::{ParseTerm, TermError, TermParser},
};

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum ProseError {
    #[error("label already used")]
    ConflictingRegionLabel,
    #[error("invalid indentation")]
    InvalidIndentation,
    #[error("unexpected character")]
    UnexpectedCharacter,
    #[error("no matching region start")]
    UnmatchedRegionEnd,
    #[error("no matching region end")]
    UnmatchedRegionStart,
    #[error(transparent)]
    Term(#[from] TermError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProseChunk<'a> {
    Text(&'a str),
    RegionStart { id: usize },
    RegionEnd { id: usize },
    TermEvent(ParseTerm<'a>),
}

impl<'a> ProseChunk<'a> {
    pub fn from_nonempty_text(text: &'a str) -> Option<Self> {
        if text.is_empty() {
            return None;
        }

        Some(ProseChunk::Text(text))
    }
}

struct Region {
    id: usize,
    marker_start: usize,
}

struct Block<'a> {
    region: Region,
    indent: Option<&'a str>,
}

#[derive(Clone, Copy)]
enum Marker<'a> {
    EscapedAt {
        marker_start: usize,
    },
    PointStart {
        marker_start: usize,
        body_start: usize,
    },
    RegionOpenStart {
        label: Option<&'a str>,
        marker_start: usize,
        body_start: usize,
    },
    RegionClose {
        label: Option<&'a str>,
        marker_start: usize,
        marker_end: usize,
    },
    BlockOpenStart {
        marker_start: usize,
        body_start: usize,
    },
    BlockClose {
        marker_start: usize,
    },
    Invalid {
        marker_start: usize,
    },
}

impl Marker<'_> {
    pub fn marker_start(&self) -> usize {
        *match self {
            Marker::EscapedAt { marker_start }
            | Marker::PointStart { marker_start, .. }
            | Marker::RegionOpenStart { marker_start, .. }
            | Marker::RegionClose { marker_start, .. }
            | Marker::BlockOpenStart { marker_start, .. }
            | Marker::BlockClose { marker_start }
            | Marker::Invalid { marker_start } => marker_start,
        }
    }

    pub fn block_depth(self, open_blocks: usize) -> Option<usize> {
        match self {
            Marker::BlockOpenStart { .. } => Some(open_blocks),
            Marker::BlockClose { .. } if open_blocks > 0 => Some(open_blocks - 1),
            _ => None,
        }
    }
}

enum MarkerContinuation<'a> {
    PointEnd {
        id: usize,
    },
    RegionOpenEnd {
        id: usize,
        label: Option<&'a str>,
        marker_start: usize,
    },
    BlockOpenEnd {
        id: usize,
        marker_start: usize,
    },
}

#[derive(Clone, Copy)]
enum TextBoundary<'a> {
    EndOfFile,
    Inline(Marker<'a>),
    Block {
        prefix_start: usize,
        block_depth: usize,
        marker: Marker<'a>,
    },
}

#[derive(Clone, Copy)]
enum BlockLineEnd {
    EndOfFile,
    InlineMarker,
    BlockMarker,
}

enum ProseParserState<'a> {
    Idle,
    EmittingText {
        boundary: TextBoundary<'a>,
    },
    EmittingFacts {
        parser: TermParser<'a>,
        base: usize,
        cont: MarkerContinuation<'a>,
    },
    FinishingBlockEnd,
    Done,
    Failed,
}

pub struct ProseParser<'a> {
    source: &'a str,
    offset: usize,
    next_region_id: usize,
    labeled_region_map: HashMap<&'a str, Region>,
    anonymous_region_stack: Vec<Region>,
    block_stack: Vec<Block<'a>>,
    state: ProseParserState<'a>,
}

impl<'a> ProseParser<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            offset: 0,
            next_region_id: 0,
            labeled_region_map: HashMap::new(),
            anonymous_region_stack: Vec::new(),
            block_stack: Vec::new(),
            state: ProseParserState::Idle,
        }
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    fn find_boundary(&self) -> Result<TextBoundary<'a>, usize> {
        if let Some(marker_start) = self.source[self.offset..]
            .find('@')
            .map(|offset| self.offset + offset)
        {
            let marker = scan_marker(self.source, marker_start);
            if let Some(block_depth) = marker.block_depth(self.block_stack.len()) {
                let prefix_start = match self.source[..marker_start]
                    .rfind(|character| !matches!(character, ' ' | '\t'))
                {
                    None => 0,
                    Some(previous) if self.source.as_bytes()[previous] == b'\n' => previous + 1,
                    Some(previous) => {
                        return Err(previous);
                    }
                };

                Ok(TextBoundary::Block {
                    prefix_start: prefix_start.max(self.offset),
                    block_depth,
                    marker,
                })
            } else {
                Ok(TextBoundary::Inline(marker))
            }
        } else {
            Ok(TextBoundary::EndOfFile)
        }
    }

    fn step(&mut self) -> Result<Option<ProseChunk<'a>>, ProseError> {
        match self.state {
            ProseParserState::Idle => match self.find_boundary() {
                Ok(boundary) => {
                    self.state = ProseParserState::EmittingText { boundary };
                    Ok(None)
                }
                Err(err_offset) => {
                    self.offset = err_offset;
                    Err(ProseError::UnexpectedCharacter)
                }
            },
            ProseParserState::EmittingText { boundary } => self.step_text(boundary),
            ProseParserState::EmittingFacts { .. } => self.step_facts(),
            ProseParserState::FinishingBlockEnd => {
                self.skip_block_marker_line_end()?;
                self.state = ProseParserState::Idle;
                Ok(None)
            }
            ProseParserState::Done | ProseParserState::Failed => unreachable!(),
        }
    }

    fn new_region_id(&mut self) -> usize {
        let id = self.next_region_id;
        self.next_region_id += 1;
        id
    }

    fn step_text(
        &mut self,
        boundary: TextBoundary<'a>,
    ) -> Result<Option<ProseChunk<'a>>, ProseError> {
        let end = boundary.text_end(self.source.len());
        let text = if self.block_stack.is_empty() {
            if self.offset < end {
                let text = &self.source[self.offset..end];
                self.offset = end;
                Some(text)
            } else {
                None
            }
        } else if self.offset < end
            || (matches!(boundary, TextBoundary::Inline(_))
                && is_start_of_line(self.source, self.offset))
        {
            self.parse_block_line(end, boundary.block_line_end())?
        } else {
            None
        };

        if let Some(text) = text {
            return Ok(ProseChunk::from_nonempty_text(text));
        }

        match boundary {
            TextBoundary::EndOfFile => {
                let unclosed = self
                    .anonymous_region_stack
                    .iter()
                    .chain(self.labeled_region_map.values())
                    .chain(self.block_stack.iter().map(|block| &block.region))
                    .max_by_key(|region| region.id)
                    .map(|region| region.marker_start);

                if let Some(opening) = unclosed {
                    self.offset = opening;
                    return Err(ProseError::UnmatchedRegionStart);
                }

                self.state = ProseParserState::Done;

                Ok(None)
            }
            TextBoundary::Inline(marker) => {
                self.offset = marker.marker_start();
                self.state = ProseParserState::Idle;
                self.step_marker(marker)
            }
            TextBoundary::Block {
                prefix_start,
                block_depth,
                marker,
            } => {
                self.parse_block_indent(prefix_start..marker.marker_start(), block_depth)?;
                self.offset = marker.marker_start();
                self.state = ProseParserState::Idle;
                self.step_marker(marker)
            }
        }
    }

    fn step_marker(&mut self, marker: Marker<'a>) -> Result<Option<ProseChunk<'a>>, ProseError> {
        match marker {
            Marker::EscapedAt { marker_start } => {
                self.offset = marker_start + 2;
                Ok(ProseChunk::from_nonempty_text(
                    &self.source[marker_start + 1..marker_start + 2],
                ))
            }
            Marker::PointStart { body_start, .. } => {
                let id = self.begin_parsing_point(body_start)?;
                Ok(Some(ProseChunk::RegionStart { id }))
            }
            Marker::RegionOpenStart {
                label,
                marker_start,
                body_start,
            } => {
                let id = self.begin_parsing_region(label, marker_start, body_start)?;
                Ok(Some(ProseChunk::RegionStart { id }))
            }
            Marker::RegionClose {
                label,
                marker_start,
                marker_end,
            } => {
                self.offset = marker_end;
                let region = match label {
                    Some(label) => self.labeled_region_map.remove(label),
                    None => self.anonymous_region_stack.pop(),
                };

                let Some(region) = region else {
                    self.offset = marker_start;
                    return Err(ProseError::UnmatchedRegionEnd);
                };

                Ok(Some(ProseChunk::RegionEnd { id: region.id }))
            }
            Marker::BlockOpenStart {
                marker_start,
                body_start,
            } => {
                let id = self.new_region_id();
                self.begin_parsing_facts(
                    body_start,
                    b'|',
                    MarkerContinuation::BlockOpenEnd { id, marker_start },
                );
                Ok(Some(ProseChunk::RegionStart { id }))
            }
            Marker::BlockClose { marker_start } => {
                self.offset = marker_start + 2;
                let region = self.block_stack.pop().map(|block| block.region);

                let Some(region) = region else {
                    self.offset = marker_start;
                    return Err(ProseError::UnmatchedRegionEnd);
                };

                self.state = ProseParserState::FinishingBlockEnd;
                Ok(Some(ProseChunk::RegionEnd { id: region.id }))
            }
            Marker::Invalid { marker_start } => {
                self.offset = marker_start;
                Err(ProseError::UnexpectedCharacter)
            }
        }
    }

    fn begin_parsing_point(&mut self, body_start: usize) -> Result<usize, ProseError> {
        let id = self.new_region_id();
        self.begin_parsing_facts(body_start, b'}', MarkerContinuation::PointEnd { id });
        Ok(id)
    }

    fn begin_parsing_region(
        &mut self,
        label: Option<&'a str>,
        marker_start: usize,
        body_start: usize,
    ) -> Result<usize, ProseError> {
        if label.is_some_and(|label| self.labeled_region_map.contains_key(label)) {
            self.offset = marker_start;
            return Err(ProseError::ConflictingRegionLabel);
        }

        let id = self.new_region_id();
        self.begin_parsing_facts(
            body_start,
            b'|',
            MarkerContinuation::RegionOpenEnd {
                id,
                label,
                marker_start,
            },
        );

        Ok(id)
    }

    fn begin_parsing_facts(
        &mut self,
        body_start: usize,
        terminator: u8,
        cont: MarkerContinuation<'a>,
    ) {
        self.offset = body_start;
        self.state = ProseParserState::EmittingFacts {
            parser: TermParser::new(&self.source[body_start..], terminator),
            base: body_start,
            cont,
        };
    }

    fn step_facts(&mut self) -> Result<Option<ProseChunk<'a>>, ProseError> {
        let ProseParserState::EmittingFacts { parser, base, .. } = &mut self.state else {
            unreachable!()
        };
        let base = *base;

        match parser.next() {
            Some(Ok(event)) => return Ok(Some(ProseChunk::TermEvent(event))),
            Some(Err(error)) => {
                self.offset = base + parser.offset();
                return Err(error.into());
            }
            None => {}
        }

        let ProseParserState::EmittingFacts {
            parser,
            cont: after,
            ..
        } = std::mem::replace(&mut self.state, ProseParserState::Idle)
        else {
            unreachable!()
        };
        self.offset = base + parser.offset();
        self.finish_parsing_facts(after)
    }

    fn finish_parsing_facts(
        &mut self,
        cont: MarkerContinuation<'a>,
    ) -> Result<Option<ProseChunk<'a>>, ProseError> {
        match cont {
            MarkerContinuation::PointEnd { id } => Ok(Some(ProseChunk::RegionEnd { id })),
            MarkerContinuation::RegionOpenEnd {
                id,
                label,
                marker_start,
            } => {
                let region = Region { id, marker_start };
                match label {
                    Some(label) => {
                        self.labeled_region_map.insert(label, region);
                    }
                    None => self.anonymous_region_stack.push(region),
                }
                Ok(None)
            }
            MarkerContinuation::BlockOpenEnd { id, marker_start } => {
                self.skip_block_marker_line_end()?;
                self.block_stack.push(Block {
                    region: Region { id, marker_start },
                    indent: None,
                });
                Ok(None)
            }
        }
    }

    fn parse_block_line(
        &mut self,
        end: usize,
        ending: BlockLineEnd,
    ) -> Result<Option<&'a str>, ProseError> {
        if !is_start_of_line(self.source, self.offset) {
            let line_end = scan_end_of_line(self.source, self.offset, end);
            let text = &self.source[self.offset..line_end];
            self.offset = line_end;
            return Ok(Some(text));
        }

        let line_start = self.offset;
        while self.offset < end && matches!(self.source.as_bytes()[self.offset], b' ' | b'\t') {
            self.offset += 1;
        }

        if self.offset < end {
            if let Some(newline_end) = scan_newline_sequence(self.source, self.offset, end) {
                let text = &self.source[self.offset..newline_end];
                self.offset = newline_end;
                return Ok(Some(text));
            }

            let line_end = scan_end_of_line(self.source, self.offset, end);
            let text = self.parse_block_indent(line_start..line_end, self.block_stack.len())?;
            self.offset = line_end;
            return Ok(Some(text));
        }

        self.offset = line_start;
        match ending {
            BlockLineEnd::InlineMarker => {
                let text = self.parse_block_indent(line_start..end, self.block_stack.len())?;
                self.offset = end;
                Ok(Some(text))
            }
            BlockLineEnd::EndOfFile | BlockLineEnd::BlockMarker => {
                self.offset = end;
                Ok(None)
            }
        }
    }

    fn parse_block_indent(
        &mut self,
        range: Range<usize>,
        depth: usize,
    ) -> Result<&'a str, ProseError> {
        let mut start = range.start;

        for block in &mut self.block_stack[..depth] {
            let indent = block
                .indent
                .get_or_insert_with(|| {
                    let end = (start..range.end)
                        .find(|offset| !matches!(self.source.as_bytes()[*offset], b' ' | b'\t'))
                        .unwrap_or(range.end);
                    &self.source[start..end]
                })
                .as_bytes();

            if range.end - start < indent.len()
                || self.source.as_bytes()[start..start + indent.len()] != *indent
            {
                self.offset = range.start;
                return Err(ProseError::InvalidIndentation);
            }
            start += indent.len();
        }

        Ok(&self.source[start..range.end])
    }

    fn skip_block_marker_line_end(&mut self) -> Result<(), ProseError> {
        while matches!(self.source.as_bytes().get(self.offset), Some(b' ' | b'\t')) {
            self.offset += 1;
        }

        if self.offset == self.source.len() {
            return Ok(());
        }

        if let Some(newline_end) =
            scan_newline_sequence(self.source, self.offset, self.source.len())
        {
            self.offset = newline_end;
            return Ok(());
        }

        Err(ProseError::UnexpectedCharacter)
    }
}

impl<'a> Iterator for ProseParser<'a> {
    type Item = Result<ProseChunk<'a>, ProseError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if matches!(
                self.state,
                ProseParserState::Done | ProseParserState::Failed
            ) {
                return None;
            }

            match self.step() {
                Ok(Some(event)) => return Some(Ok(event)),
                Ok(None) => {}
                Err(error) => {
                    self.state = ProseParserState::Failed;
                    return Some(Err(error));
                }
            }
        }
    }
}

impl FusedIterator for ProseParser<'_> {}

impl TextBoundary<'_> {
    fn text_end(self, document_end: usize) -> usize {
        match self {
            Self::EndOfFile => document_end,
            Self::Inline(marker) => marker.marker_start(),
            Self::Block { prefix_start, .. } => prefix_start,
        }
    }

    fn block_line_end(self) -> BlockLineEnd {
        match self {
            Self::EndOfFile => BlockLineEnd::EndOfFile,
            Self::Inline(_) => BlockLineEnd::InlineMarker,
            Self::Block { .. } => BlockLineEnd::BlockMarker,
        }
    }
}

fn is_start_of_line(source: &str, offset: usize) -> bool {
    offset == 0 || source.as_bytes().get(offset - 1) == Some(&b'\n')
}

fn scan_marker<'a>(source: &'a str, offset: usize) -> Marker<'a> {
    let rest = &source[offset + 1..];
    if rest.starts_with('@') {
        Marker::EscapedAt {
            marker_start: offset,
        }
    } else if rest.starts_with('{') {
        Marker::PointStart {
            marker_start: offset,
            body_start: offset + 2,
        }
    } else if rest.starts_with('<') {
        Marker::RegionOpenStart {
            label: None,
            marker_start: offset,
            body_start: offset + 2,
        }
    } else if rest.starts_with('[') {
        Marker::BlockOpenStart {
            marker_start: offset,
            body_start: offset + 2,
        }
    } else if rest.starts_with('>') {
        Marker::RegionClose {
            label: None,
            marker_start: offset,
            marker_end: offset + 2,
        }
    } else if rest.starts_with(']') {
        Marker::BlockClose {
            marker_start: offset,
        }
    } else if rest
        .as_bytes()
        .first()
        .is_some_and(|byte| is_ident_start(*byte))
    {
        let label_start = offset + 1;
        let label_end = scan_ident(source, label_start);
        let label = Some(&source[label_start..label_end]);
        match source.as_bytes().get(label_end) {
            Some(b'<') => Marker::RegionOpenStart {
                label,
                marker_start: offset,
                body_start: label_end + 1,
            },
            Some(b'>') => Marker::RegionClose {
                label,
                marker_start: offset,
                marker_end: label_end + 1,
            },
            _ => Marker::Invalid {
                marker_start: offset,
            },
        }
    } else {
        Marker::Invalid {
            marker_start: offset,
        }
    }
}

fn scan_newline_sequence(source: &str, offset: usize, end: usize) -> Option<usize> {
    if source.as_bytes().get(offset) == Some(&b'\n') {
        Some(offset + 1)
    } else if offset + 1 < end
        && source.as_bytes().get(offset) == Some(&b'\r')
        && source.as_bytes().get(offset + 1) == Some(&b'\n')
    {
        Some(offset + 2)
    } else {
        None
    }
}

fn scan_end_of_line(source: &str, mut offset: usize, end: usize) -> usize {
    while offset < end {
        if let Some(newline_end) = scan_newline_sequence(source, offset, end) {
            return newline_end;
        }
        let character = source[offset..]
            .chars()
            .next()
            .expect("offset before end of range");
        offset += character.len_utf8();
    }
    end
}
