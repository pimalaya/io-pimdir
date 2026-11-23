//! # Chunks
//!
//! The boundaries extraction records per indexed body (SEARCH §6): the
//! `body` and `note` fields split into paragraphs at blank-line runs and
//! packed greedily into chunks of at most [`CHUNK_MAX`] UTF-8 bytes. A
//! snippet is cut from a hit's chunks, and an embedding index embeds the
//! same ones without extracting again.

use alloc::vec::Vec;
use core::ops::Range;

/// The most UTF-8 bytes a chunk spans.
pub const CHUNK_MAX: usize = 1024;

/// The chunks of one field's text, as half-open UTF-8 byte ranges into
/// it, in order (SEARCH §6): [`Chunks`] collected.
pub fn chunks(text: &str) -> Vec<Range<usize>> {
    Chunks::new(text).collect()
}

/// The chunks of one field's text, as half-open UTF-8 byte ranges into
/// it, in order (SEARCH §6), each cut when asked for, so a reader stopping
/// at one scans no further.
///
/// A paragraph is what lies between blank-line runs, a run being a line
/// break followed by one or more lines of spaces and tabs alone, each
/// ended by a line break. Paragraphs pack greedily, a chunk running from
/// its first paragraph's start to its last one's end, and one that does
/// not fit starts the next chunk. A paragraph longer than [`CHUNK_MAX`]
/// is cut into pieces, each ending just after the last space, tab, CR or
/// LF within its first [`CHUNK_MAX`] bytes, else at the last character
/// boundary within them; every piece but the last is a chunk of its own,
/// the last packed as a paragraph.
pub struct Chunks<'t> {
    text: &'t str,
    paragraphs: Paragraphs<'t>,
    /// The chunk being packed.
    current: Option<Range<usize>>,
    /// The paragraph being cut into pieces, from the first byte left.
    cut: Option<(usize, Range<usize>)>,
    /// A piece cut right after the chunk it closed.
    piece: Option<Range<usize>>,
}

impl<'t> Chunks<'t> {
    /// The chunks of `text`.
    pub fn new(text: &'t str) -> Self {
        Self {
            text,
            paragraphs: Paragraphs::new(text.as_bytes()),
            current: None,
            cut: None,
            piece: None,
        }
    }
}

impl Iterator for Chunks<'_> {
    type Item = Range<usize>;

    fn next(&mut self) -> Option<Range<usize>> {
        if let Some(piece) = self.piece.take() {
            return Some(piece);
        }
        loop {
            let (start, paragraph) = match self.cut.take() {
                Some(cut) => cut,
                None => match self.paragraphs.next() {
                    Some(paragraph) => (paragraph.start, paragraph),
                    None => return self.current.take(),
                },
            };
            if let Some(chunk) = &mut self.current
                && paragraph.end - chunk.start <= CHUNK_MAX
            {
                chunk.end = paragraph.end;
                continue;
            }
            let closed = self.current.take();

            if paragraph.end - start <= CHUNK_MAX {
                self.current = Some(start..paragraph.end);
                match closed {
                    Some(closed) => return Some(closed),
                    None => continue,
                }
            }

            let bytes = self.text.as_bytes();
            let window = &bytes[start..start + CHUNK_MAX];
            let cut = match window
                .iter()
                .rposition(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
            {
                Some(space) => start + space + 1,
                None => {
                    let mut cut = start + CHUNK_MAX;
                    while !self.text.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    cut
                }
            };
            self.cut = Some((cut, paragraph));
            match closed {
                Some(closed) => {
                    self.piece = Some(start..cut);
                    return Some(closed);
                }
                None => return Some(start..cut),
            }
        }
    }
}

/// The paragraphs of a text: the spans between its blank-line runs, empty
/// ones left out.
struct Paragraphs<'t> {
    bytes: &'t [u8],
    /// Where the next paragraph starts.
    at: usize,
    /// Where the scan for the next line break resumes.
    scan: usize,
    /// Whether the last paragraph was given.
    done: bool,
}

impl<'t> Paragraphs<'t> {
    fn new(bytes: &'t [u8]) -> Self {
        Self {
            bytes,
            at: 0,
            scan: 0,
            done: false,
        }
    }
}

impl Iterator for Paragraphs<'_> {
    type Item = Range<usize>;

    fn next(&mut self) -> Option<Range<usize>> {
        let bytes = self.bytes;
        while self.scan < bytes.len() {
            match line_end(&bytes[self.scan..]) {
                Some(skip) => self.scan += skip,
                None => {
                    self.scan = bytes.len();
                    break;
                }
            }
            match blank_run(bytes, self.scan) {
                Some(end) => {
                    let paragraph = (self.scan > self.at).then_some(self.at..self.scan);
                    self.at = end;
                    self.scan = end;
                    if paragraph.is_some() {
                        return paragraph;
                    }
                }
                None => self.scan += 1,
            }
        }
        if self.done {
            return None;
        }
        self.done = true;
        (bytes.len() > self.at).then_some(self.at..bytes.len())
    }
}

/// The offset of the first CR or LF of `bytes`, read a word at a time.
fn line_end(bytes: &[u8]) -> Option<usize> {
    const ONES: u64 = u64::from_ne_bytes([1; 8]);
    const HIGHS: u64 = u64::from_ne_bytes([0x80; 8]);
    let (words, rest) = bytes.as_chunks::<8>();
    let mut at = 0;
    for word in words {
        let word = u64::from_le_bytes(*word);
        let lf = word ^ (ONES * u64::from(b'\n'));
        let cr = word ^ (ONES * u64::from(b'\r'));
        let zero = (lf.wrapping_sub(ONES) & !lf | cr.wrapping_sub(ONES) & !cr) & HIGHS;
        if zero != 0 {
            return Some(at + zero.trailing_zeros() as usize / 8);
        }
        at += 8;
    }
    rest.iter()
        .position(|b| matches!(b, b'\n' | b'\r'))
        .map(|skip| at + skip)
}

/// The end of the blank-line run starting at `at`, if one does: a line
/// break, then one or more lines of spaces and tabs each ended by one.
fn blank_run(bytes: &[u8], at: usize) -> Option<usize> {
    let mut end = line_break(bytes, at)?;
    let mut lines = 0;
    loop {
        let mut next = end;
        while matches!(bytes.get(next), Some(b' ' | b'\t')) {
            next += 1;
        }
        match line_break(bytes, next) {
            Some(after) => {
                end = after;
                lines += 1;
            }
            None => break,
        }
    }
    (lines > 0).then_some(end)
}

/// The end of the line break (LF, or CR LF) starting at `at`, if one does.
fn line_break(bytes: &[u8], at: usize) -> Option<usize> {
    match (bytes.get(at), bytes.get(at + 1)) {
        (Some(b'\n'), _) => Some(at + 1),
        (Some(b'\r'), Some(b'\n')) => Some(at + 2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::search::chunk::*;

    fn paragraphs(bytes: &[u8]) -> Vec<Range<usize>> {
        Paragraphs::new(bytes).collect()
    }

    #[test]
    fn a_bare_carriage_return_breaks_no_line() {
        assert_eq!(paragraphs(b"a\r\rb").first(), Some(&(0..4)));
        assert_eq!(paragraphs(b"a\r\n\r\nb"), [0..1, 5..6]);
        assert_eq!(chunks("a\r\n\r\nb").first(), Some(&(0..6)));
    }

    #[test]
    fn a_line_end_read_a_word_at_a_time_is_the_first_cr_or_lf() {
        for len in 0..40 {
            for at in 0..=len {
                for end in *b"\r\n" {
                    let mut bytes = alloc::vec![0x8a_u8; len];
                    if at < len {
                        bytes[at] = end;
                        bytes[len - 1] = b'\n';
                    }
                    let want = bytes.iter().position(|b| matches!(b, b'\n' | b'\r'));
                    assert_eq!(line_end(&bytes), want, "{bytes:?}");
                }
            }
        }
    }
}
