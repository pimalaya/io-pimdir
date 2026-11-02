//! The streamed walk of a message against the whole-body derivation
//! (Annex A.1): for the same octets, the attachment mark and the
//! invitation a writer reads as the body streams by must be those
//! `derive` reads off the body held whole, however the octets are cut.
//!
//! A table pins the cases the format names (base64, quoted-printable,
//! folded `UID` lines, nested multiparts, a calendar part disposed as an
//! attachment or inline, a truncated message, one cut inside a line
//! break, a header line holding a `\r`, no `UID`); a property then builds
//! messages from those pieces at random, cut anywhere.

use io_pimdir::summary::{
    PimdirSummary,
    mail::{
        self,
        attachments::{self, CALENDAR_LINE_MAX, PimdirAttachmentWalk, PimdirParts},
    },
};
use proptest::prelude::*;

/// The mark and invitation `derive` reads off the whole body.
fn derived(body: &[u8]) -> (bool, Option<String>) {
    match mail::derive(body).summary {
        Some(PimdirSummary::Mail(mail)) => (mail.attachment.unwrap(), mail.invitation),
        _ => unreachable!("a message derives a mail summary"),
    }
}

/// The walk fed `body` in `chunks`-octet pieces, read only while it
/// says it still reads, as a writer streaming a blob feeds it.
fn streamed(body: &[u8], chunk: usize) -> PimdirParts {
    let mut walk = PimdirAttachmentWalk::new();
    for piece in body.chunks(chunk.max(1)) {
        if walk.done() {
            break;
        }
        walk.feed(piece);
    }
    walk.finish_parts()
}

/// Asserts the walk, whole, byte by byte and cut at `cut`, reads what
/// the derivation does, answering the invitation.
fn agrees(body: &[u8], cut: usize) -> Option<String> {
    let (mark, invitation) = derived(body);
    let whole = attachments::parts(body);
    assert_eq!(
        whole.attachment,
        mark,
        "mark of {:?}",
        String::from_utf8_lossy(body)
    );
    assert_eq!(
        whole.invitation,
        invitation,
        "invitation of {:?}",
        String::from_utf8_lossy(body)
    );
    for chunk in [1, 7, 64 * 1024] {
        assert_eq!(streamed(body, chunk), whole, "fed {chunk} octets at a time");
    }
    let cut = cut.min(body.len());
    let mut walk = PimdirAttachmentWalk::new();
    walk.feed(&body[..cut]);
    walk.feed(&body[cut..]);
    assert_eq!(walk.finish_parts(), whole, "cut at {cut}");
    invitation
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (at, byte)| n | u32::from(*byte) << (16 - 8 * at));
        for at in 0..4 {
            if at <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * at) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Wraps `text` into lines of `width` with `eol`.
fn wrapped(text: &str, width: usize, eol: &str) -> String {
    text.as_bytes()
        .chunks(width)
        .map(|line| String::from_utf8_lossy(line).into_owned() + eol)
        .collect()
}

const ICS: &str = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:evt-1@example.org\r\nSUMMARY:Stand-up\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

#[test]
fn the_walk_reads_the_invitation_the_derivation_reads() {
    let b64 = format!(
        "Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nhi\r\n--b\r\nContent-Type: text/calendar; method=REQUEST\r\nContent-Transfer-Encoding: base64\r\n\r\n{}--b--\r\n",
        wrapped(&base64(ICS.as_bytes()), 76, "\r\n")
    );
    assert_eq!(
        agrees(b64.as_bytes(), 50).as_deref(),
        Some("evt-1@example.org")
    );

    let qp = b"Content-Type: text/calendar\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nBEGIN:VCALENDAR\r\nUID:caf=C3=A9-=3D1\r\nEND:VCALENDAR\r\n";
    assert_eq!(agrees(qp, 40).as_deref(), Some("café-=1"));

    let folded =
        b"Content-Type: text/calendar\n\nBEGIN:VEVENT\nUID;X-A=1:abc\n def\n\tghi\nSUMMARY:x\n";
    assert_eq!(agrees(folded, 20).as_deref(), Some("abcdefghi"));

    let folded_last = b"Content-Type: text/calendar\r\n\r\nUID:abc\r\n def";
    assert_eq!(agrees(folded_last, 33).as_deref(), Some("abcdef"));

    let nested = b"Content-Type: multipart/mixed; boundary=out\r\n\r\n\
--out\r\nContent-Type: multipart/alternative; boundary=in\r\n\r\n\
--in\r\nContent-Type: text/plain\r\n\r\nhi\r\n\
--in\r\nContent-Type: TEXT/Calendar; charset=utf-8\r\n\r\nUID:inline-1\r\n\
--in--\r\n\
--out\r\nContent-Type: text/calendar\r\nContent-Disposition: attachment; filename=invite.ics\r\n\r\nUID:attached-2\r\n\
--out--\r\n";
    assert_eq!(agrees(nested, 100).as_deref(), Some("inline-1"));

    let attached = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n\
--b\r\nContent-Type: text/calendar\r\n\r\nSUMMARY:no uid\r\n\
--b\r\nContent-Type: text/calendar\r\nContent-Disposition: attachment\r\n\r\nUID:second\r\n\
--b--\r\n";
    assert_eq!(agrees(attached, 60).as_deref(), Some("second"));

    let truncated = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n\
--b\r\nContent-Type: text/plain\r\n\r\nhi\r\n\
--b\r\nContent-Type: text/calendar\r\n\r\nUID:cut\r\n";
    assert_eq!(agrees(truncated, 70), None, "a part no delimiter ends");

    let cut_inner = b"Content-Type: multipart/mixed; boundary=out\r\n\r\n\
--out\r\nContent-Type: multipart/mixed; boundary=in\r\n\r\n\
--in\r\nContent-Type: text/calendar\r\n\r\nUID:cut-inner\r\n\
--out\r\nContent-Type: text/calendar\r\n\r\nUID:after\r\n--out--\r\n";
    assert_eq!(agrees(cut_inner, 90).as_deref(), Some("after"));

    let cut_line_feed = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n\
--b\r\nContent-Type: text/calendar\r\nContent-Disposition: attachment\r\n\r\nUID:last\r\n\
--b--\r";
    assert!(derived(cut_line_feed).0);
    assert_eq!(
        agrees(cut_line_feed, 0).as_deref(),
        Some("last"),
        "a delimiter cut before its line feed"
    );

    // NOTE: the multipart and its bare file are read only when the header
    // block goes on past the line holding a `\r`.
    let stray_cr = b"Subject: cr\r\n\r\r\n\
Content-Type: multipart/mixed; boundary=b\r\n\r\n\
--b\r\nContent-Type: text/calendar\r\n\r\nUID:after-cr\r\n\
--b\r\nContent-Type: application/pdf\r\n\r\n%PDF\r\n--b--\r\n";
    assert!(
        derived(stray_cr).0,
        "a line holding a `\\r` is no empty one"
    );
    assert_eq!(agrees(stray_cr, 50).as_deref(), Some("after-cr"));

    let header_cut = b"Content-Type: text/calendar\r\nContent-Disposition: attachment\r";
    assert!(derived(header_cut).0);
    assert_eq!(agrees(header_cut, 20), None);

    let foreign = b"Content-Type: text/calendar\r\nContent-Transfer-Encoding: base64\r\n\r\nVUlEOng=\r\n!\r\n";
    assert_eq!(agrees(foreign, 10), None, "base64 over a foreign byte");

    let none = b"Content-Type: text/calendar\r\n\r\nBEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n";
    assert_eq!(agrees(none, 30), None);

    let plain = b"Subject: hi\r\nContent-Type: text/plain\r\n\r\nUID:not-a-calendar\r\n";
    assert_eq!(agrees(plain, 30), None);

    let invalid = b"Content-Type: text/calendar\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nUID:\xc3=A9\xff=C3\r\n";
    agrees(invalid, 40);

    let soft = b"Content-Type: text/calendar\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nUID:ab=\r\ncd=4\r\n";
    agrees(soft, 70);

    // NOTE: a multipart message has no section, so its own disposition
    // makes no mark (Annex A.1).
    let disposed = b"Content-Type: multipart/mixed; boundary=b\r\nContent-Disposition: attachment\r\n\r\n--b\r\n\r\nx\r\n--b--\r\n";
    assert!(!derived(disposed).0);
    agrees(disposed, 10);
}

#[test]
fn a_uid_past_the_line_bound_is_not_read() {
    let mut body = b"Content-Type: text/calendar\r\n\r\nUID:".to_vec();
    body.extend(core::iter::repeat_n(b'x', CALENDAR_LINE_MAX));
    body.extend_from_slice(b"\r\nUID:short\r\n");
    assert_eq!(attachments::parts(&body).invitation, None);

    // NOTE: the bound ends the search as a read UID does: a later part's
    // is not taken for it.
    let mut body = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/calendar\r\n\r\nUID:".to_vec();
    body.extend(core::iter::repeat_n(b'y', 2 * CALENDAR_LINE_MAX));
    body.extend_from_slice(b"\r\n--b\r\nContent-Type: text/calendar\r\n\r\nUID:later\r\n--b--\r\n");
    assert_eq!(attachments::parts(&body).invitation, None);
}

/// A small generator of messages, driven by a seed.
struct Builder {
    state: u64,
    eol: &'static str,
}

impl Builder {
    fn next(&mut self, below: u64) -> u64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state % below.max(1)
    }

    fn calendar(&mut self) -> Vec<u8> {
        let eol = self.eol;
        let mut lines = vec![String::from("BEGIN:VCALENDAR")];
        if self.next(4) != 0 {
            let name = ["UID", "uid", "UID;X-P=\"a\"", "UIDX", "X-UID"][self.next(5) as usize];
            let value = ["evt-1@example.org", "café", "a:b", "", "x y"][self.next(5) as usize];
            let mut line = format!("{name}:{value}");
            // NOTE: folded at a random point, by a space or a tab.
            if self.next(2) == 0 && line.len() > 2 {
                let at = 1 + self.next(line.len() as u64 - 1) as usize;
                if line.is_char_boundary(at) {
                    let fold = if self.next(2) == 0 { " " } else { "\t" };
                    line.insert_str(at, &format!("{eol}{fold}"));
                }
            }
            lines.push(line);
        }
        if self.next(3) == 0 {
            lines.push(String::from("UID:second-uid"));
        }
        lines.push(String::from("END:VCALENDAR"));
        let mut text = lines.join(eol) + eol;
        if self.next(5) == 0 {
            text.push(' ');
        }
        text.into_bytes()
    }

    /// A leaf part: its header lines and its encoded body.
    fn leaf(&mut self) -> (String, Vec<u8>) {
        let eol = self.eol;
        let (media, raw) = match self.next(3) {
            0 => ("text/plain", b"hello UID:not-this\xff".to_vec()),
            1 => ("application/pdf", b"%PDF-1.4".to_vec()),
            _ => ("text/calendar; method=REQUEST", self.calendar()),
        };
        let mut headers = format!("Content-Type: {media}{eol}");
        if self.next(3) == 0 {
            headers += &format!("Content-Disposition: attachment; filename=f{eol}");
        }
        let body = match self.next(3) {
            0 => {
                headers += &format!("Content-Transfer-Encoding: base64{eol}");
                let mut text = wrapped(&base64(&raw), [4, 8, 76][self.next(3) as usize], eol);
                if self.next(8) == 0 {
                    text.insert(0, '*');
                }
                text.into_bytes()
            }
            1 => {
                headers += &format!("Content-Transfer-Encoding: Quoted-Printable{eol}");
                let mut out = Vec::new();
                for byte in &raw {
                    match self.next(6) {
                        0 => out.extend_from_slice(format!("={byte:02X}").as_bytes()),
                        1 => out.extend_from_slice(format!("={byte:02x}").as_bytes()),
                        2 if *byte != b'\n' => {
                            out.push(*byte);
                            out.extend_from_slice(b"=");
                            out.extend_from_slice(eol.as_bytes());
                        }
                        _ => out.push(*byte),
                    }
                }
                if self.next(6) == 0 {
                    out.extend_from_slice(b"=4");
                }
                out
            }
            _ => raw,
        };
        (headers, body)
    }

    /// An entity at `depth`: a leaf, or a multipart of one to three.
    fn entity(&mut self, depth: usize) -> Vec<u8> {
        let eol = self.eol;
        if depth < 3 && self.next(3) != 0 {
            let boundary = format!("b{depth}{}", self.next(100));
            let mut out = format!(
                "Content-Type: multipart/{}; boundary=\"{boundary}\"{eol}{eol}",
                ["mixed", "alternative", "related"][self.next(3) as usize]
            )
            .into_bytes();
            if self.next(3) == 0 {
                out.extend_from_slice(format!("preamble UID:no{eol}").as_bytes());
            }
            for _ in 0..1 + self.next(3) {
                out.extend_from_slice(format!("--{boundary}{eol}").as_bytes());
                out.extend(self.entity(depth + 1));
                out.extend_from_slice(eol.as_bytes());
            }
            if self.next(5) != 0 {
                out.extend_from_slice(format!("--{boundary}--{eol}").as_bytes());
            }
            return out;
        }
        let (headers, body) = self.leaf();
        let mut out = headers.into_bytes();
        out.extend_from_slice(eol.as_bytes());
        out.extend(body);
        out
    }
}

/// The message the property built from `seed`, cut at `truncate`.
fn built(seed: u64, crlf: bool, truncate: Option<usize>) -> Vec<u8> {
    let mut builder = Builder {
        state: seed,
        eol: if crlf { "\r\n" } else { "\n" },
    };
    let mut body = format!("Subject: built{0}Message-ID: <m@x>{0}", builder.eol).into_bytes();
    body.extend(builder.entity(0));
    if let Some(at) = truncate {
        body.truncate(at.min(body.len()));
    }
    body
}

#[test]
fn a_built_message_cut_before_its_last_line_feed_reads_alike() {
    // NOTE: the property's case of 2026-11-01, the message cut between
    // the `\r` and the `\n` of its closing delimiter.
    let body = built(1794814323191950817, true, Some(1242));
    assert!(body.ends_with(b"\r\n--b047--\r"));
    assert!(derived(&body).0);
    agrees(&body, 0);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn a_built_message_reads_alike_streamed_or_whole(
        seed in 1u64..,
        crlf in any::<bool>(),
        truncate in proptest::option::of(0usize..4096),
        cut in 0usize..4096,
    ) {
        agrees(&built(seed, crlf, truncate), cut);
    }
}
