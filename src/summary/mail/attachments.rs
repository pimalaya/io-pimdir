//! # A message's attachments
//!
//! The parts a message stands for as files (STORAGE §14.3, Annex A.7):
//! exactly those making the attachment mark (Annex A.1), each with its
//! IMAP section (RFC 3501 §6.4.5), its name, its media type and its
//! decoded size.
//!
//! The walk is the mark's own, boundary by boundary down to
//! [`MIME_DEPTH_MAX`] and through [`MIME_ENTITIES_MAX`] entities, but fed
//! the body a chunk at a time, so a writer reads a stored body from its
//! blob file with bounded memory: a header block of at most [`HEADER_MAX`]
//! octets per part, the first [`LINE_HEAD`] octets of any other line, one
//! boundary per open level, and at most [`ATTACHMENTS_MAX`] summaries. A
//! part's bytes are counted, never kept.
//!
//! The same walk reads the message's invitation (Annex A.1), the first
//! `UID` of the first `text/calendar` part stating one, its transfer
//! encoding undone and its lines unfolded, keeping one unfolded line of
//! at most [`CALENDAR_LINE_MAX`] octets: a writer that read the body as
//! it streamed, never whole, learns it in the same pass
//! ([`PimdirAttachmentWalk::finish_parts`]).

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use super::{
    MIME_DEPTH_MAX, MIME_ENTITIES_MAX, boundary, charset_decode, decode, delimits, field,
    header_fields, hex_pair,
};
use crate::summary::file::PimdirFileSummary;

/// The most attachments one message stands for, the first the walk marks
/// in document order (STORAGE §14.3): a reader finding this many reads the
/// rest from the body.
pub const ATTACHMENTS_MAX: usize = 1000;

/// The octets of a part's header block the walk keeps; the rest of a
/// longer block is not read.
pub const HEADER_MAX: usize = 64 * 1024;

/// The octets of a body line the walk keeps to tell a boundary
/// delimiter from content; a boundary that would not fit opens nothing.
pub const LINE_HEAD: usize = 1024;

/// The octets of one unfolded line of a `text/calendar` part the walk
/// keeps while it looks for the invitation: a `UID` line longer than
/// this is not read, and the message reads as stating none.
pub const CALENDAR_LINE_MAX: usize = 64 * 1024;

/// What the walk of a message reads off its parts.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirParts {
    /// The parts carrying `Content-Disposition: attachment`, in document
    /// order, as the summaries of the files standing for them.
    pub attachments: Vec<PimdirFileSummary>,
    /// The attachment mark (Annex A.1): whether a part, or the message
    /// itself, carries `Content-Disposition: attachment`, as
    /// [`derive`](super::derive) reads it with the body.
    pub attachment: bool,
    /// The `UID` the first `text/calendar` part stating one carries
    /// (Annex A.1), as [`derive`](super::derive) reads it.
    pub invitation: Option<String>,
}

/// The parts of `body` carrying `Content-Disposition: attachment`, in
/// document order, as the summaries of the files standing for them
/// (Annex A.7), `part` set to each one's section.
pub fn attachments(body: &[u8]) -> Vec<PimdirFileSummary> {
    let mut walk = PimdirAttachmentWalk::new();
    walk.feed(body);
    walk.finish()
}

/// What the walk reads off `body` held in memory: its attachments, its
/// attachment mark and its invitation.
pub fn parts(body: &[u8]) -> PimdirParts {
    let mut walk = PimdirAttachmentWalk::new();
    walk.feed(body);
    walk.finish_parts()
}

/// The walk of a message fed a chunk at a time ([`attachments`] for a
/// body held in memory).
///
/// Feed it the stored octets in order with [`feed`](Self::feed), as
/// long as [`done`](Self::done) says it still reads, then take the
/// summaries with [`finish`](Self::finish), or them, the attachment mark
/// and the invitation with [`finish_parts`](Self::finish_parts). A
/// single-part message is done once its header block is read, a
/// `text/calendar` one once its `UID` is.
#[derive(Debug)]
pub struct PimdirAttachmentWalk {
    /// The first octets of the line being read, its terminator excluded.
    head: Vec<u8>,
    /// What the rest of the line being read goes to.
    route: Route,
    /// A `\r` ending the octets fed so far, a terminator if `\n` follows.
    cr: bool,
    /// Whether octets of a line not yet ended were fed.
    open: bool,
    /// The multipart levels open, outermost first.
    frames: Vec<Frame>,
    /// What the lines of the current entity are.
    mode: Mode,
    /// The summaries recorded so far, `size` set as each part ends.
    found: Vec<PimdirFileSummary>,
    /// The `text/calendar` part under way, read for its `UID`.
    calendar: Option<Calendar>,
    /// The invitation a part that ended gave: `Some(None)` for a `UID`
    /// too long to read, which ends the search as a read one does.
    invitation: Option<Option<String>>,
    /// Whether the message's own header block disposes it as an
    /// attachment: a multipart one has no section, so no summary.
    disposed: bool,
    /// The entities read, the message's own counted: a part beginning
    /// past [`MIME_ENTITIES_MAX`] is skipped, its delimiters still read.
    read: usize,
}

/// Where the octets of a line past its head go.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum Route {
    /// Kept in the head until the line ends or the head is full.
    Undecided,
    /// A header line: kept up to the block's limit.
    Keep,
    /// Content of an attachment: counted.
    Count,
    /// Nothing reads them.
    Drop,
    /// The line was a delimiter, handled already.
    Done,
}

/// One open multipart level.
#[derive(Debug)]
struct Frame {
    /// `--` and the boundary.
    delimiter: Vec<u8>,
    /// The section its children number under: empty for the message,
    /// else the multipart part's section and a dot.
    prefix: String,
    /// The children numbered so far.
    children: usize,
    /// Where the summaries of the child under way start, `None` before
    /// the first delimiter: a child no delimiter ends is no part.
    child_start: Option<usize>,
    /// Whether the invitation was read before the child under way began,
    /// so a child cut short takes back only one it gave.
    invited_before: bool,
}

/// What the lines of the current entity are.
#[derive(Debug)]
enum Mode {
    /// Its header block, to the first empty line.
    Header {
        block: Vec<u8>,
        section: Option<String>,
        depth: usize,
    },
    /// A body that is no multipart opened, counted when it is an
    /// attachment: the summary's index and its counter.
    Leaf(Option<(usize, Counter)>),
    /// A preamble or an epilogue, which nothing reads.
    Skip,
}

impl Default for PimdirAttachmentWalk {
    fn default() -> Self {
        Self::new()
    }
}

impl PimdirAttachmentWalk {
    /// A walk at the start of a message.
    pub fn new() -> Self {
        Self {
            head: Vec::new(),
            route: Route::Undecided,
            cr: false,
            open: false,
            frames: Vec::new(),
            mode: Mode::Header {
                block: Vec::new(),
                section: None,
                depth: 0,
            },
            found: Vec::new(),
            calendar: None,
            invitation: None,
            disposed: false,
            read: 1,
        }
    }

    /// Whether nothing the walk has yet to read can add an attachment or
    /// end one, nor change the invitation: no multipart level is open, no
    /// attachment is being counted and no calendar part is still read,
    /// so a reader may stop feeding it.
    pub fn done(&self) -> bool {
        self.frames.is_empty()
            && match &self.mode {
                Mode::Header { .. } => false,
                Mode::Leaf(counter) => {
                    counter.is_none() && self.calendar.as_ref().is_none_or(Calendar::settled)
                }
                Mode::Skip => true,
            }
    }

    /// Reads the next octets of the message.
    pub fn feed(&mut self, mut chunk: &[u8]) {
        while !chunk.is_empty() && !self.done() {
            match chunk.iter().position(|byte| *byte == b'\n') {
                Some(at) => {
                    self.segment(&chunk[..at]);
                    let terminator = if self.cr { 2 } else { 1 };
                    self.cr = false;
                    self.end_line(terminator);
                    chunk = &chunk[at + 1..];
                }
                None => {
                    self.segment(chunk);
                    chunk = &[];
                }
            }
        }
    }

    /// Ends the message, answering its attachments in document order.
    pub fn finish(self) -> Vec<PimdirFileSummary> {
        self.finish_parts().attachments
    }

    /// Ends the message, answering its attachments in document order and
    /// its invitation.
    pub fn finish_parts(mut self) -> PimdirParts {
        // NOTE: a `\r` ending the message ends its last line as the
        // derivation reads a line, at a line feed or at the end alike
        // (`parts`, `unfold`): a delimiter cut before its `\n` still
        // delimits, and a header line drops it. A part's content keeps
        // it, as the derivation's slice of the part does.
        if self.cr {
            self.cr = false;
            if self.route == Route::Undecided
                && let Some((level, close)) = self.delimiter()
            {
                self.delimit(level, close);
                self.route = Route::Done;
            } else if !matches!(self.mode, Mode::Header { .. }) {
                self.content(b"\r");
            }
        }
        if self.open {
            self.end_line(0);
        }
        if matches!(self.mode, Mode::Header { .. }) {
            self.end_headers();
        }
        self.close_leaf(true);
        // NOTE: a child the outermost level never ended is no part, as
        // the walk for the mark slices them (Annex A.1), so neither its
        // attachments nor its invitation count.
        if let Some(frame) = self.frames.first()
            && let Some(start) = frame.child_start
        {
            self.found.truncate(start);
            if !frame.invited_before {
                self.invitation = None;
            }
        }
        let attachment = self.disposed || !self.found.is_empty();
        PimdirParts {
            attachments: self.found,
            attachment,
            invitation: self.invitation.flatten(),
        }
    }

    /// Octets of the line under way, its `\n` excluded.
    fn segment(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.open = true;
        if self.cr {
            self.cr = false;
            self.content(b"\r");
        }
        match bytes.split_last() {
            Some((b'\r', rest)) => {
                self.cr = true;
                self.content(rest);
            }
            _ => self.content(bytes),
        }
    }

    /// Routes content octets of the line under way.
    fn content(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        match self.route {
            Route::Undecided => {
                let room = self.head_cap().saturating_sub(self.head.len());
                let (kept, rest) = bytes.split_at(room.min(bytes.len()));
                self.head.extend_from_slice(kept);
                if !rest.is_empty() || self.head.len() >= self.head_cap() {
                    self.decide();
                    self.content(rest);
                }
            }
            Route::Keep => {
                let room = self.head_cap().saturating_sub(self.head.len());
                self.head.extend_from_slice(&bytes[..room.min(bytes.len())]);
            }
            Route::Count => {
                if let Mode::Leaf(Some((_, counter))) = &mut self.mode {
                    counter.bytes(bytes);
                }
                if let Some(calendar) = &mut self.calendar {
                    calendar.bytes(bytes);
                }
            }
            Route::Drop | Route::Done => {}
        }
    }

    /// The octets of a line kept: a header block's room, else
    /// [`LINE_HEAD`].
    fn head_cap(&self) -> usize {
        match &self.mode {
            Mode::Header { block, .. } => HEADER_MAX.saturating_sub(block.len()).max(LINE_HEAD),
            _ => LINE_HEAD,
        }
    }

    /// Decides a line longer than its head: a delimiter, or content of
    /// the entity under way.
    fn decide(&mut self) {
        if let Some((level, close)) = self.delimiter() {
            self.delimit(level, close);
            self.route = Route::Done;
            return;
        }
        self.route = match &mut self.mode {
            Mode::Header { .. } => Route::Keep,
            Mode::Leaf(counter) if counter.is_some() || self.calendar.is_some() => {
                if let Some((_, counter)) = counter {
                    counter.bytes(&self.head);
                }
                if let Some(calendar) = &mut self.calendar {
                    calendar.bytes(&self.head);
                }
                Route::Count
            }
            Mode::Leaf(_) | Mode::Skip => Route::Drop,
        };
    }

    /// Ends the line under way, `terminator` its octets (0 at the end of
    /// the message).
    fn end_line(&mut self, terminator: u64) {
        if self.route == Route::Undecided {
            match self.delimiter() {
                Some((level, close)) => self.delimit(level, close),
                None => self.line(terminator),
            }
        } else {
            match self.route {
                Route::Keep => self.header_line(),
                Route::Count => {
                    if let Mode::Leaf(Some((_, counter))) = &mut self.mode {
                        counter.end_line(terminator);
                    }
                    if let Some(calendar) = &mut self.calendar {
                        calendar.end_line(terminator);
                    }
                }
                _ => {}
            }
        }
        self.head.clear();
        self.route = Route::Undecided;
        self.open = false;
    }

    /// A whole line that is no delimiter, held in the head.
    fn line(&mut self, terminator: u64) {
        match &mut self.mode {
            Mode::Header { .. } if self.head.is_empty() => self.end_headers(),
            Mode::Header { .. } => self.header_line(),
            Mode::Leaf(counter) => {
                if let Some((_, counter)) = counter {
                    counter.bytes(&self.head);
                    counter.end_line(terminator);
                }
                if let Some(calendar) = &mut self.calendar {
                    calendar.bytes(&self.head);
                    calendar.end_line(terminator);
                }
            }
            Mode::Skip => {}
        }
    }

    /// Adds the line in the head to the header block, within its limit.
    fn header_line(&mut self) {
        if let Mode::Header { block, .. } = &mut self.mode {
            let room = HEADER_MAX.saturating_sub(block.len());
            let line = &self.head[..self.head.len().min(room)];
            block.extend_from_slice(line);
            // NOTE: a line its own `\r` ends breaks at `\r\n`, so the
            // block reads that `\r` as the line's own, as the raw block
            // the derivation unfolds does, never as its line break.
            if block.len() < HEADER_MAX {
                let terminator: &[u8] = match line.last() {
                    Some(b'\r') => b"\r\n",
                    _ => b"\n",
                };
                block.extend_from_slice(terminator);
            }
        }
    }

    /// The level whose delimiter the head opens with, outermost first as
    /// the mark's walk slices them, and whether it closes that level.
    fn delimiter(&self) -> Option<(usize, bool)> {
        self.frames
            .iter()
            .enumerate()
            .find_map(|(level, frame)| Some((level, delimits(&self.head, &frame.delimiter)?)))
    }

    /// A delimiter of the level `level`: the part under way ends, and the
    /// next starts unless `close` ends the level.
    fn delimit(&mut self, level: usize, close: bool) {
        if matches!(self.mode, Mode::Header { .. }) {
            self.end_headers();
        }
        self.close_leaf(false);

        // NOTE: a level inside the part ending here has its own child
        // under way cut short, which is no part either.
        if let Some(inner) = self.frames.get(level + 1) {
            if let Some(start) = inner.child_start {
                self.found.truncate(start);
                if !inner.invited_before {
                    self.invitation = None;
                }
            }
            self.frames.truncate(level + 1);
        }

        if close {
            self.frames.truncate(level);
            self.mode = Mode::Skip;
            return;
        }

        let found = self.found.len();
        let invited = self.invitation.is_some();
        let frame = &mut self.frames[level];
        frame.children += 1;
        frame.child_start = Some(found);
        frame.invited_before = invited;

        // NOTE: a part past the bound is no entity read, but the levels
        // stay open: a part they never end is still no part.
        if self.read >= MIME_ENTITIES_MAX {
            self.mode = Mode::Skip;
            return;
        }
        self.read += 1;
        self.mode = Mode::Header {
            block: Vec::new(),
            section: Some(format!("{}{}", frame.prefix, frame.children)),
            depth: level + 1,
        };
    }

    /// Ends the current entity's header block: records it when it is an
    /// attachment, and opens it when it is a multipart.
    fn end_headers(&mut self) {
        let Mode::Header {
            block,
            section,
            depth,
        } = core::mem::replace(&mut self.mode, Mode::Skip)
        else {
            return;
        };
        let headers = header_fields(&block);

        let attached = field(&headers, "content-disposition")
            .and_then(|value| value.split(';').next())
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("attachment"));
        if depth == 0 && attached {
            self.disposed = true;
        }
        let content_type = field(&headers, "content-type");
        let opened = match content_type.and_then(boundary) {
            Some(boundary) if depth < MIME_DEPTH_MAX && boundary.len() + 4 <= LINE_HEAD => {
                Some(boundary)
            }
            _ => None,
        };
        // NOTE: the message itself has no section but as a single part,
        // numbered 1 (RFC 3501 §6.4.5).
        let section = section.or_else(|| opened.is_none().then(|| "1".to_string()));

        let recorded = match section.as_ref() {
            Some(section) if attached && self.found.len() < ATTACHMENTS_MAX => {
                self.found.push(PimdirFileSummary {
                    name: name(&headers),
                    media_type: content_type
                        .and_then(|value| value.split(';').next())
                        .map(|media| media.trim().to_lowercase())
                        .filter(|media| !media.is_empty()),
                    size: None,
                    part: Some(section.clone()),
                });
                Some(self.found.len() - 1)
            }
            _ => None,
        };

        match opened {
            Some(boundary) => {
                self.frames.push(Frame {
                    delimiter: format!("--{boundary}").into_bytes(),
                    prefix: section.map(|section| section + ".").unwrap_or_default(),
                    children: 0,
                    child_start: None,
                    invited_before: false,
                });
                self.mode = Mode::Skip;
            }
            None => {
                let encoding = field(&headers, "content-transfer-encoding")
                    .map(|value| value.trim().to_ascii_lowercase());
                // NOTE: read whether or not it is disposed as an
                // attachment, as the derivation reads it (Annex A.1).
                let calendar = content_type
                    .and_then(|value| value.split(';').next())
                    .is_some_and(|media| media.trim().eq_ignore_ascii_case("text/calendar"));
                if calendar && self.invitation.is_none() {
                    self.calendar = Some(Calendar::new(encoding.as_deref()));
                }
                self.mode =
                    Mode::Leaf(recorded.map(|index| (index, Counter::new(encoding.as_deref()))));
            }
        }
    }

    /// Ends the attachment being counted, its size set; a pending line
    /// break counts only at the end of the message, any other belonging
    /// to the delimiter that follows (RFC 2046 §5.1.1). A calendar part
    /// ends too, its `UID`, if it read one, the invitation.
    fn close_leaf(&mut self, at_end: bool) {
        if let Mode::Leaf(Some((index, counter))) = &mut self.mode {
            let size = counter.size(at_end);
            if let Some(found) = self.found.get_mut(*index) {
                found.size = Some(size);
            }
            self.mode = Mode::Leaf(None);
        }
        if let Some(calendar) = self.calendar.take()
            && let Some(invitation) = calendar.finish()
        {
            self.invitation = Some(invitation);
        }
    }
}

/// The name an attachment states (Annex A.7): the `Content-Disposition`
/// `filename`, else the `Content-Type` `name`, RFC 2231 and RFC 2047
/// decoded; empty when neither states one.
fn name(headers: &[(String, String)]) -> String {
    let stated = |header: &str, param: &str| {
        field(headers, header).and_then(|value| parameter(value, param))
    };
    stated("content-disposition", "filename")
        .or_else(|| stated("content-type", "name"))
        .map(|name| match name.contains("=?") {
            true => decode(&name),
            false => name,
        })
        .unwrap_or_default()
}

/// One parameter of a structured header value, RFC 2231 extended and
/// continued forms assembled and decoded, the extended form winning.
fn parameter(value: &str, wanted: &str) -> Option<String> {
    let mut plain: Option<String> = None;
    let mut extended: Option<String> = None;
    // NOTE: the continued segments by number, whether each is encoded.
    let mut segments: Vec<(u32, bool, String)> = Vec::new();

    for (name, raw) in parameters(value) {
        let Some(rest) = name.strip_prefix(wanted) else {
            continue;
        };
        match rest {
            "" => plain = Some(raw),
            "*" => extended = Some(raw),
            rest => {
                let Some(rest) = rest.strip_prefix('*') else {
                    continue;
                };
                let (number, encoded) = match rest.strip_suffix('*') {
                    Some(number) => (number, true),
                    None => (rest, false),
                };
                if let Ok(number) = number.parse::<u32>() {
                    segments.push((number, encoded, raw));
                }
            }
        }
    }

    if let Some(raw) = extended {
        return Some(extended_value(&[(true, raw)]));
    }
    if !segments.is_empty() {
        segments.sort_by_key(|(number, _, _)| *number);
        let parts: Vec<(bool, String)> = segments
            .into_iter()
            .map(|(_, encoded, raw)| (encoded, raw))
            .collect();
        return Some(extended_value(&parts));
    }
    plain
}

/// An RFC 2231 value from its segments in order: the first encoded one
/// carries `charset'language'`, every encoded one is percent-decoded,
/// and the octets read under the charset.
fn extended_value(segments: &[(bool, String)]) -> String {
    let mut charset = String::new();
    let mut octets: Vec<u8> = Vec::new();

    for (index, (encoded, raw)) in segments.iter().enumerate() {
        if !encoded {
            octets.extend_from_slice(raw.as_bytes());
            continue;
        }
        let mut text = raw.as_str();
        if index == 0 {
            let mut fields = raw.splitn(3, '\'');
            if let (Some(stated), Some(_), Some(rest)) =
                (fields.next(), fields.next(), fields.next())
            {
                charset = stated.to_string();
                text = rest;
            }
        }
        let bytes = text.as_bytes();
        let mut at = 0;
        while at < bytes.len() {
            match bytes[at] {
                b'%' if at + 2 < bytes.len() => match hex_pair(bytes[at + 1], bytes[at + 2]) {
                    Some(byte) => {
                        octets.push(byte);
                        at += 3;
                    }
                    None => {
                        octets.push(b'%');
                        at += 1;
                    }
                },
                byte => {
                    octets.push(byte);
                    at += 1;
                }
            }
        }
    }

    // NOTE: Annex A.7 names no reading for a charset no WHATWG label
    // names: the octets read as UTF-8, as with no charset stated.
    charset_decode(&charset, &octets)
        .unwrap_or_else(|| String::from_utf8_lossy(&octets).into_owned())
}

/// The parameters of a structured header value, its leading value
/// dropped: names lowercased, quoted values unquoted, split at the
/// semicolons outside quotes.
fn parameters(value: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut escaped = false;
    let mut first = true;

    let mut push = |current: &mut String, first: &mut bool| {
        if !core::mem::take(first)
            && let Some((name, raw)) = current.split_once('=')
        {
            out.push((name.trim().to_lowercase(), unquote(raw.trim())));
        }
        current.clear();
    };

    for char in value.chars() {
        if escaped {
            current.push(char);
            escaped = false;
            continue;
        }
        match char {
            '\\' if quoted => {
                current.push(char);
                escaped = true;
            }
            '"' => {
                quoted = !quoted;
                current.push(char);
            }
            ';' if !quoted => push(&mut current, &mut first),
            _ => current.push(char),
        }
    }
    push(&mut current, &mut first);

    out
}

/// A parameter value with its quotes and their escapes removed.
fn unquote(raw: &str) -> String {
    let Some(inner) = raw
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return raw.to_string();
    };
    let mut out = String::with_capacity(inner.len());
    let mut escaped = false;
    for char in inner.chars() {
        match (escaped, char) {
            (false, '\\') => escaped = true,
            _ => {
                out.push(char);
                escaped = false;
            }
        }
    }
    out
}

/// Counts the octets a part decodes to, its transfer encoding undone,
/// without keeping them.
#[derive(Debug)]
struct Counter {
    encoding: Encoding,
    /// The decoded octets counted.
    count: u64,
    /// The line break ending the last line, counted once another line
    /// follows.
    pending: u64,
    /// Whether the line under way has had octets.
    started: bool,
    /// Base64: the alphabet characters read.
    sextets: u64,
    /// Quoted-printable: the trailing whitespace of the line under way,
    /// dropped at a hard break (RFC 2045 §6.7, rule 3).
    blanks: u64,
    /// Quoted-printable: within an `=` escape.
    escape: Escape,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum Encoding {
    Identity,
    Base64,
    QuotedPrintable,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum Escape {
    None,
    Equals,
    Hex(u8),
}

impl Counter {
    fn new(encoding: Option<&str>) -> Self {
        Self {
            encoding: match encoding {
                Some("base64") => Encoding::Base64,
                Some("quoted-printable") => Encoding::QuotedPrintable,
                _ => Encoding::Identity,
            },
            count: 0,
            pending: 0,
            started: false,
            sextets: 0,
            blanks: 0,
            escape: Escape::None,
        }
    }

    /// Octets of the line under way.
    fn bytes(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.start();
        match self.encoding {
            Encoding::Identity => self.count += bytes.len() as u64,
            Encoding::Base64 => {
                self.sextets += bytes
                    .iter()
                    .filter(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/'))
                    .count() as u64;
            }
            Encoding::QuotedPrintable => {
                for byte in bytes {
                    self.quoted(*byte);
                }
            }
        }
    }

    /// One quoted-printable octet; a malformed escape counts as written.
    fn quoted(&mut self, byte: u8) {
        match self.escape {
            Escape::None => match byte {
                b'=' => self.escape = Escape::Equals,
                b' ' | b'\t' => {
                    self.count += 1;
                    self.blanks += 1;
                }
                _ => {
                    self.count += 1;
                    self.blanks = 0;
                }
            },
            // NOTE: whitespace after a soft break's `=` is transport
            // padding.
            Escape::Equals if matches!(byte, b' ' | b'\t') => {}
            Escape::Equals if byte.is_ascii_hexdigit() => self.escape = Escape::Hex(byte),
            Escape::Equals => {
                self.escape = Escape::None;
                self.count += 1;
                self.blanks = 0;
                self.quoted(byte);
            }
            Escape::Hex(high) => {
                self.escape = Escape::None;
                self.blanks = 0;
                match hex_pair(high, byte) {
                    Some(_) => self.count += 1,
                    None => {
                        self.count += 2;
                        self.quoted(byte);
                    }
                }
            }
        }
    }

    /// Counts the line break before a line's first octet.
    fn start(&mut self) {
        if !self.started {
            self.started = true;
            self.count += core::mem::take(&mut self.pending);
        }
    }

    /// Ends a line whose break is `terminator` octets.
    fn end_line(&mut self, terminator: u64) {
        self.start();
        self.started = false;
        match self.encoding {
            Encoding::Identity => self.pending = terminator,
            Encoding::Base64 => {}
            Encoding::QuotedPrintable => match self.escape {
                // NOTE: a soft line break joins the next line.
                Escape::Equals => {
                    self.escape = Escape::None;
                    self.blanks = 0;
                }
                escape => {
                    if let Escape::Hex(_) = escape {
                        self.count += 2;
                        self.blanks = 0;
                    }
                    self.escape = Escape::None;
                    self.count -= core::mem::take(&mut self.blanks);
                    self.pending = terminator;
                }
            },
        }
    }

    /// The decoded size, the pending line break counted at the end of
    /// the message only.
    fn size(&self, at_end: bool) -> u64 {
        match self.encoding {
            Encoding::Base64 => self.sextets * 6 / 8,
            _ => {
                self.count
                    + if at_end && !self.started {
                        self.pending
                    } else {
                        0
                    }
            }
        }
    }
}

/// Reads a `text/calendar` part for its first `UID` as its octets go by,
/// the way the derivation reads the part whole (Annex A.1): the transfer
/// encoding undone (base64 decoding to nothing over a foreign byte), the
/// lines unfolded, each split at its first colon, the name before any
/// parameter. One unfolded line of at most [`CALENDAR_LINE_MAX`] octets is
/// kept.
#[derive(Debug)]
struct Calendar {
    encoding: Encoding,
    /// Quoted-printable: within an `=` escape.
    escape: Escape,
    /// Base64: the bits read and not yet an octet, and how many.
    buffer: u32,
    bits: u32,
    /// Base64: a byte outside the alphabet was read.
    foreign: bool,
    /// The decoded octets of the line under way, its `\n` excluded.
    line: Vec<u8>,
    /// Whether the line under way outgrew the bound.
    line_over: bool,
    /// The unfolded line under way, `None` before the first line.
    unfolded: Option<String>,
    /// Whether the unfolded line under way outgrew the bound.
    unfolded_over: bool,
    /// The `UID` read: `Some(None)` for one too long to read.
    uid: Option<Option<String>>,
}

impl Calendar {
    fn new(encoding: Option<&str>) -> Self {
        Self {
            encoding: match encoding {
                Some("base64") => Encoding::Base64,
                Some("quoted-printable") => Encoding::QuotedPrintable,
                _ => Encoding::Identity,
            },
            escape: Escape::None,
            buffer: 0,
            bits: 0,
            foreign: false,
            line: Vec::new(),
            line_over: false,
            unfolded: None,
            unfolded_over: false,
            uid: None,
        }
    }

    /// Whether nothing left of the part can change what it gives: its
    /// `UID` is read, and no foreign base64 byte can take it back.
    fn settled(&self) -> bool {
        self.uid.is_some() && self.encoding != Encoding::Base64
    }

    /// Raw octets of the part, line breaks included.
    fn bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            match self.encoding {
                Encoding::Identity => self.decoded(*byte),
                Encoding::QuotedPrintable => self.quoted(*byte),
                Encoding::Base64 => self.sextet(*byte),
            }
        }
    }

    /// The line break ending a raw line, `terminator` its octets.
    fn end_line(&mut self, terminator: u64) {
        match terminator {
            2 => self.bytes(b"\r\n"),
            1 => self.bytes(b"\n"),
            _ => {}
        }
    }

    /// One base64 character: whitespace and padding skipped, anything
    /// else outside the alphabet foreign.
    fn sextet(&mut self, byte: u8) {
        if self.foreign {
            return;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b' ' | b'\t' | b'\r' | b'\n' => return,
            _ => {
                self.foreign = true;
                return;
            }
        };
        self.buffer = (self.buffer << 6) | u32::from(value);
        self.bits += 6;
        if self.bits >= 8 {
            self.bits -= 8;
            self.decoded((self.buffer >> self.bits) as u8);
            self.buffer &= (1 << self.bits) - 1;
        }
    }

    /// One quoted-printable octet: an `=` and two hexadecimal digits are
    /// the octet they spell, any other `=` itself, as RFC 2045 rule 1
    /// reads with no soft break undone, as the derivation does.
    fn quoted(&mut self, byte: u8) {
        match self.escape {
            Escape::None if byte == b'=' => self.escape = Escape::Equals,
            Escape::None => self.decoded(byte),
            Escape::Equals if byte.is_ascii_hexdigit() => self.escape = Escape::Hex(byte),
            Escape::Equals => {
                self.escape = Escape::None;
                self.decoded(b'=');
                self.quoted(byte);
            }
            Escape::Hex(high) => {
                self.escape = Escape::None;
                match hex_pair(high, byte) {
                    Some(octet) => self.decoded(octet),
                    None => {
                        self.decoded(b'=');
                        self.decoded(high);
                        self.quoted(byte);
                    }
                }
            }
        }
    }

    /// One decoded octet.
    fn decoded(&mut self, byte: u8) {
        if self.uid.is_some() {
            return;
        }
        match byte {
            b'\n' => self.end_physical(),
            _ if self.line.len() < CALENDAR_LINE_MAX => self.line.push(byte),
            _ => self.line_over = true,
        }
    }

    /// Ends a decoded line: a line opening with a space or a tab
    /// continues the one before it, that one character dropped (RFC 5545
    /// §3.1), any other ends it.
    fn end_physical(&mut self) {
        let line = core::mem::take(&mut self.line);
        let over = core::mem::take(&mut self.line_over);
        let text = String::from_utf8_lossy(&line);
        let text = text.strip_suffix('\r').unwrap_or(&text);

        match (text.strip_prefix([' ', '\t']), &mut self.unfolded) {
            (Some(rest), Some(unfolded)) => {
                if unfolded.len() + rest.len() <= CALENDAR_LINE_MAX {
                    unfolded.push_str(rest);
                } else {
                    self.unfolded_over = true;
                }
                self.unfolded_over |= over;
            }
            _ => {
                self.end_unfolded();
                self.unfolded = Some(text.to_string());
                self.unfolded_over = over;
            }
        }
    }

    /// Ends an unfolded line, its value the `UID` when its name is one.
    /// One too long to keep is a `UID` too long to read when what it
    /// kept names one.
    fn end_unfolded(&mut self) {
        let Some(line) = self.unfolded.take() else {
            return;
        };
        if self.uid.is_some() {
            return;
        }
        if core::mem::take(&mut self.unfolded_over) {
            let name = line.split([':', ';']).next().unwrap_or_default();
            if line.contains([':', ';']) && name.eq_ignore_ascii_case("UID") {
                self.uid = Some(None);
            }
            return;
        }
        if let Some((name, value)) = line.split_once(':')
            && name
                .split(';')
                .next()
                .unwrap_or_default()
                .eq_ignore_ascii_case("UID")
        {
            self.uid = Some(Some(value.to_string()));
        }
    }

    /// Ends the part: the `UID` it gives, `None` when it states none or
    /// its base64 decodes to nothing.
    fn finish(mut self) -> Option<Option<String>> {
        if self.foreign {
            return None;
        }
        match self.escape {
            Escape::None => {}
            Escape::Equals => self.decoded(b'='),
            Escape::Hex(high) => {
                self.decoded(b'=');
                self.decoded(high);
            }
        }
        if self.uid.is_none() {
            self.end_physical();
            self.end_unfolded();
        }
        self.uid
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;
    use crate::summary::{PimdirSummary, mail::derive};

    fn summary(name: &str, media_type: Option<&str>, size: u64, part: &str) -> PimdirFileSummary {
        PimdirFileSummary {
            name: name.into(),
            media_type: media_type.map(Into::into),
            size: Some(size),
            part: Some(part.into()),
        }
    }

    /// Every way of cutting `body` into two chunks walks it alike.
    fn walked(body: &[u8]) -> Vec<PimdirFileSummary> {
        let whole = attachments(body);
        for cut in 0..=body.len() {
            let mut walk = PimdirAttachmentWalk::new();
            walk.feed(&body[..cut]);
            walk.feed(&body[cut..]);
            assert_eq!(walk.finish(), whole, "cut at {cut}");
        }
        let mut walk = PimdirAttachmentWalk::new();
        for byte in body {
            walk.feed(core::slice::from_ref(byte));
        }
        assert_eq!(walk.finish(), whole, "byte by byte");
        whole
    }

    #[test]
    fn the_attachments_of_a_mixed_message_are_its_disposed_parts() {
        let body = b"Content-Type: multipart/mixed; boundary=\"outer\"\r\n\r\n\
preamble\r\n\
--outer\r\n\
Content-Type: multipart/alternative; boundary=inner\r\n\r\n\
--inner\r\nContent-Type: text/plain\r\n\r\nhi\r\n\
--inner\r\nContent-Type: text/html\r\n\r\n<p>hi</p>\r\n\
--inner--\r\n\
--outer\r\n\
Content-Type: image/png\r\nContent-Disposition: inline; filename=logo.png\r\n\r\nPNG\r\n\
--outer\r\n\
Content-Type: Application/PDF; name=\"ignored.pdf\"\r\n\
Content-Disposition: attachment;\r\n filename=\"invoice.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\r\n\
JVBERi0x\r\nLjQK\r\n\
--outer\r\n\
Content-Disposition: attachment; filename*=UTF-8''r%C3%A9sum%C3%A9.txt\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\r\n\
caf=C3=A9  \r\nsoft=\r\nly\r\n\
--outer--\r\nepilogue\r\n";

        assert_eq!(
            walked(body),
            vec![
                summary("invoice.pdf", Some("application/pdf"), 9, "3"),
                summary("résumé.txt", None, 13, "4"),
            ]
        );
    }

    #[test]
    fn a_single_part_message_disposed_as_an_attachment_is_section_one() {
        let body = b"Content-Type: application/octet-stream\r\n\
Content-Disposition: attachment; filename*0*=UTF-8''r%C3%A9; filename*1=\"sum\"; filename*2*=%C3%A9.bin\r\n\
Content-Transfer-Encoding: base64\r\n\r\nAAEC\r\n";

        assert_eq!(
            walked(body),
            vec![summary(
                "résumé.bin",
                Some("application/octet-stream"),
                3,
                "1"
            )]
        );
    }

    #[test]
    fn an_identity_part_counts_its_octets_but_the_break_before_the_delimiter() {
        let body = b"Content-Type: multipart/mixed; boundary=b\n\n--b\nContent-Type: text/plain\n\nbody\n--b\nContent-Disposition: attachment; filename=\"=?UTF-8?Q?caf=C3=A9?=.txt\"\nContent-Type: text/plain; charset=iso-8859-1\n\nline one\r\nline two\n--b--\n";

        assert_eq!(
            walked(body),
            vec![summary("café.txt", Some("text/plain"), 18, "2")]
        );
    }

    #[test]
    fn a_message_without_attachments_is_done_after_its_headers() {
        let mut walk = PimdirAttachmentWalk::new();
        walk.feed(b"Subject: hi\r\nContent-Type: text/plain\r\n\r\n");
        assert!(walk.done());
        walk.feed(b"anything at all\r\n");
        assert!(walk.finish().is_empty());

        let mut walk = PimdirAttachmentWalk::new();
        walk.feed(b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n\r\nx\r\n--b--\r\n");
        assert!(walk.done(), "the message's level closed");
    }

    #[test]
    fn a_part_no_delimiter_ends_is_no_attachment() {
        let body = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Disposition: attachment; filename=a\r\n\r\nA\r\n--b\r\nContent-Disposition: attachment; filename=b\r\n\r\nB\r\n";

        assert_eq!(walked(body), vec![summary("a", None, 1, "1")]);
    }

    #[test]
    fn an_outer_delimiter_cuts_an_inner_part_short() {
        let body = b"Content-Type: multipart/mixed; boundary=out\r\n\r\n\
--out\r\nContent-Type: multipart/mixed; boundary=in\r\n\r\n\
--in\r\nContent-Disposition: attachment; filename=kept\r\n\r\nK\r\n\
--in\r\nContent-Disposition: attachment; filename=cut\r\n\r\nC\r\n\
--out\r\nContent-Disposition: attachment; filename=last\r\n\r\nL\r\n--out--\r\n";

        assert_eq!(
            walked(body),
            vec![
                summary("kept", None, 1, "1.1"),
                summary("last", None, 1, "2")
            ]
        );
    }

    #[test]
    fn a_long_line_is_counted_and_a_long_delimiter_line_still_delimits() {
        let mut body = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Disposition: attachment; filename=big\r\n\r\n".to_vec();
        body.extend(core::iter::repeat_n(b'x', 3 * LINE_HEAD));
        body.extend_from_slice(b"\r\n--b");
        body.extend(core::iter::repeat_n(b' ', 2 * LINE_HEAD));
        body.extend_from_slice(
            b"\r\nContent-Disposition: attachment; filename=next\r\n\r\nN\r\n--b--\r\n",
        );

        assert_eq!(
            walked(&body),
            vec![
                summary("big", None, 3 * LINE_HEAD as u64, "1"),
                summary("next", None, 1, "2"),
            ]
        );
    }

    #[test]
    fn the_walk_opens_no_level_past_the_depth_bound() {
        let nested = |levels: usize| {
            let mut body = Vec::new();
            for level in 0..levels {
                body.extend_from_slice(
                    format!("Content-Type: multipart/mixed; boundary=\"b{level}x\"\r\n\r\n--b{level}x\r\n")
                        .as_bytes(),
                );
            }
            body.extend_from_slice(b"Content-Disposition: attachment; filename=deep\r\n\r\nD\r\n");
            for level in (0..levels).rev() {
                body.extend_from_slice(format!("--b{level}x--\r\n").as_bytes());
            }
            body
        };
        let mark = |body: &[u8]| match derive(body).summary {
            Some(PimdirSummary::Mail(mail)) => mail.attachment,
            _ => unreachable!("a message derives a mail summary"),
        };

        let deepest = nested(MIME_DEPTH_MAX);
        assert_eq!(walked(&deepest).len(), 1);
        assert_eq!(mark(&deepest), Some(true));

        let deeper = nested(MIME_DEPTH_MAX + 1);
        assert!(attachments(&deeper).is_empty());
        assert_eq!(mark(&deeper), Some(false), "the mark agrees");
    }

    #[test]
    fn a_message_stands_for_its_first_attachments_only() {
        let mut body = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n".to_vec();
        for _ in 0..ATTACHMENTS_MAX + 5 {
            body.extend_from_slice(b"--b\r\nContent-Disposition: attachment\r\n\r\nx\r\n");
        }
        body.extend_from_slice(b"--b--\r\n");

        let found = attachments(&body);
        assert_eq!(found.len(), ATTACHMENTS_MAX);
        assert_eq!(found[ATTACHMENTS_MAX - 1].part.as_deref(), Some("1000"));
    }
}
