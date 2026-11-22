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
/// it, in order (SEARCH §6).
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
pub fn chunks(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut chunks = Vec::new();
    let mut current: Option<Range<usize>> = None;

    for paragraph in paragraphs(bytes) {
        let mut start = paragraph.start;
        loop {
            if let Some(chunk) = &mut current
                && paragraph.end - chunk.start <= CHUNK_MAX
            {
                chunk.end = paragraph.end;
                break;
            }
            chunks.extend(current.take());

            if paragraph.end - start <= CHUNK_MAX {
                current = Some(start..paragraph.end);
                break;
            }

            let window = &bytes[start..start + CHUNK_MAX];
            let cut = match window
                .iter()
                .rposition(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
            {
                Some(space) => start + space + 1,
                None => {
                    let mut cut = start + CHUNK_MAX;
                    while !text.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    cut
                }
            };
            chunks.push(start..cut);
            start = cut;
        }
    }

    chunks.extend(current);
    chunks
}

/// The paragraphs of a text: the spans between its blank-line runs, empty
/// ones left out.
fn paragraphs(bytes: &[u8]) -> Vec<Range<usize>> {
    let mut paragraphs = Vec::new();
    let mut at = 0;
    let mut scan = 0;

    while scan < bytes.len() {
        match bytes[scan..]
            .iter()
            .position(|b| matches!(b, b'\n' | b'\r'))
        {
            Some(skip) => scan += skip,
            None => break,
        }
        match blank_run(bytes, scan) {
            Some(end) => {
                if scan > at {
                    paragraphs.push(at..scan);
                }
                at = end;
                scan = end;
            }
            None => scan += 1,
        }
    }
    if bytes.len() > at {
        paragraphs.push(at..bytes.len());
    }

    paragraphs
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

    #[test]
    fn a_bare_carriage_return_breaks_no_line() {
        assert_eq!(paragraphs(b"a\r\rb").first(), Some(&(0..4)));
        assert_eq!(paragraphs(b"a\r\n\r\nb"), [0..1, 5..6]);
        assert_eq!(chunks("a\r\n\r\nb").first(), Some(&(0..6)));
    }
}
