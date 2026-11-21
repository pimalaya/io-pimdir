//! # Mail
//!
//! A query (SEARCH §8) answered over messages held in memory, with no index
//! and no store: what a client reading a Maildir or an mbox matches each
//! message by, so it answers the hit set an index holding the same
//! messages answers. The twin of [`card`](crate::search::card).
//!
//! A message is read as the indexer reads it: its fields extracted (SEARCH
//! §6), folded (§5) and split into tokens as FTS5's `unicode61` splits
//! them, its summary and address rows as the store derives them (STORAGE
//! Annex A.1, A.6), and the flags its placement carries. A term reading
//! what neither a message nor its flags hold, an account, a collection, a
//! change stamp, a thread, an item, its references, its local state
//! (`label:`, `property:`) or a person's cards, is answered by no
//! message: [`PimdirQuery::mail_terms`] names them, and a match meeting
//! one is refused.

use alloc::{string::String, vec::Vec};
use core::fmt;

use jiff::tz::TimeZone;

use crate::{
    placement::PimdirSortKey,
    search::{
        extract::{self, PimdirExtract, PimdirExtractStatus},
        fold,
        query::{
            PimdirQuery, PimdirQueryKind, PimdirQueryPredicate, PimdirQueryTerm, PimdirQueryText,
            PimdirQueryTextField,
        },
    },
    summary::{
        PimdirAddress, PimdirAddressRole, PimdirDerivation, PimdirSummary,
        mail::{self, PimdirMailSummary},
        time,
    },
};

/// One message as a query reads it: its fields as the indexer extracts
/// them, its summary and addresses as the store keeps them, and its flags.
#[derive(Clone, Debug)]
pub struct PimdirMail {
    extract: PimdirExtract,
    summary: PimdirMailSummary,
    addresses: Vec<(PimdirAddressRole, PimdirAddress)>,
    sort_key: String,
    flags: Vec<String>,
    body: bool,
}

impl PimdirMail {
    /// Reads a `message/rfc822` body as the indexer does (SEARCH §6), with
    /// the flags its placement carries.
    pub fn read(body: &[u8], flags: Vec<String>) -> Self {
        let mut mail = Self::derived(mail::derive(body), flags, true);
        let extract = PimdirExtract::mail(body);
        if extract.status != PimdirExtractStatus::Unparseable {
            mail.extract = extract;
        }
        mail
    }

    /// Reads a message from its header block alone, as a placement holding
    /// no body (SEARCH §6), with the source's size and attachment mark
    /// ([`mail::derive_meta`]).
    pub fn read_meta(
        header: &[u8],
        size: Option<u64>,
        attachment: Option<bool>,
        flags: Vec<String>,
    ) -> Self {
        Self::derived(mail::derive_meta(header, size, attachment), flags, false)
    }

    /// Reads a message from the summary and sort key a store holds of it,
    /// as a placement holding no body (SEARCH §6).
    pub fn from_summary(summary: PimdirMailSummary, sort_key: String, flags: Vec<String>) -> Self {
        let derived = PimdirDerivation {
            link_id: summary.link_id(),
            summary: Some(PimdirSummary::Mail(summary)),
            sort_key: PimdirSortKey(sort_key),
        };
        Self::derived(derived, flags, false)
    }

    fn derived(derived: PimdirDerivation, flags: Vec<String>, body: bool) -> Self {
        let summary = match derived.summary {
            Some(PimdirSummary::Mail(summary)) => summary,
            _ => PimdirMailSummary::default(),
        };
        let held = PimdirSummary::Mail(summary);
        let addresses: Vec<_> = held
            .addresses()
            .into_iter()
            .map(|(role, address)| (role, address.clone()))
            .collect();
        let extract = PimdirExtract::summary(Some(&held), &addresses);
        let PimdirSummary::Mail(summary) = held else {
            unreachable!("a mail summary");
        };

        Self {
            extract,
            summary,
            addresses,
            sort_key: derived.sort_key.0,
            flags,
            body,
        }
    }

    /// Whether an address field of `role`, any with `None`, holds the
    /// address, or one of the domain a leading `@` names.
    fn seeks(&self, role: Option<PimdirAddressRole>, text: &PimdirQueryText) -> bool {
        let wanted = text.text.to_lowercase();
        self.addresses
            .iter()
            .filter(|(held, _)| role.is_none_or(|role| role == *held))
            .any(|(_, address)| {
                let held = address.address.to_lowercase();
                match wanted.strip_prefix('@') {
                    Some(domain) => held.rsplit_once('@').is_some_and(|(_, at)| at == domain),
                    None => held == wanted,
                }
            })
    }
}

/// A term no message answers alone (`account:`, `collection:`,
/// `folder:`, `changed:`, `thread:`, `id:`, `linked:`, `person:`,
/// `label:`, `property:`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirMailMatchError {
    /// The term's first offset, in Unicode scalar values.
    pub start: usize,
    /// The offset after its last.
    pub end: usize,
}

impl fmt::Display for PimdirMailMatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Query term at {}..{} reads the store, which no message answers",
            self.start, self.end
        )
    }
}

impl core::error::Error for PimdirMailMatchError {}

impl PimdirQuery {
    /// The terms no message answers alone, in reading order: a client
    /// holding messages and no store refuses a query naming one.
    pub fn mail_terms(&self) -> Vec<&PimdirQueryTerm> {
        self.terms()
            .into_iter()
            .map(|(term, _)| term)
            .filter(|term| {
                matches!(
                    term.predicate,
                    PimdirQueryPredicate::Account(_)
                        | PimdirQueryPredicate::Collection(_)
                        | PimdirQueryPredicate::Changed { .. }
                        | PimdirQueryPredicate::Thread(_)
                        | PimdirQueryPredicate::Id(_)
                        | PimdirQueryPredicate::Linked { .. }
                        | PimdirQueryPredicate::Person(_)
                        | PimdirQueryPredicate::Label(_)
                        | PimdirQueryPredicate::Property { .. }
                )
            })
            .collect()
    }

    /// Whether each of `mails` is a hit of the query, in order (SEARCH
    /// §8), as an index holding those messages alone answers, its dates
    /// read in `zone` and back from `now` (Unix seconds). A term no
    /// message answers refuses the match.
    pub fn match_mails(
        &self,
        mails: &[PimdirMail],
        now: i64,
        zone: &TimeZone,
    ) -> Result<Vec<bool>, PimdirMailMatchError> {
        if let Some(term) = self.mail_terms().first() {
            return Err(PimdirMailMatchError {
                start: term.start,
                end: term.end,
            });
        }

        Ok(mails
            .iter()
            .map(|mail| node(self, mail, now, zone))
            .collect())
    }
}

fn node(node: &PimdirQuery, mail: &PimdirMail, now: i64, zone: &TimeZone) -> bool {
    match node {
        PimdirQuery::And(operands) => operands.iter().all(|op| self::node(op, mail, now, zone)),
        PimdirQuery::Or(operands) => operands.iter().any(|op| self::node(op, mail, now, zone)),
        PimdirQuery::Not(operand) => !self::node(operand, mail, now, zone),
        PimdirQuery::Group(operand) => self::node(operand, mail, now, zone),
        PimdirQuery::Term(term) => self::term(&term.predicate, mail, now, zone),
    }
}

fn term(predicate: &PimdirQueryPredicate, mail: &PimdirMail, now: i64, zone: &TimeZone) -> bool {
    let holds = |name: &str, text: &PimdirQueryText| fold::holds(mail.extract.column(name), text);
    match predicate {
        PimdirQueryPredicate::Text { field, text } => match field {
            PimdirQueryTextField::Any => extract::COLUMNS.iter().any(|name| holds(name, text)),
            PimdirQueryTextField::Title => holds("title", text),
            PimdirQueryTextField::Body => holds("body", text),
            PimdirQueryTextField::Attachment => holds("attachment", text),
            PimdirQueryTextField::Place => holds("place", text),
        },
        PimdirQueryPredicate::Address { role, text } if text.text.contains('@') => {
            mail.seeks(*role, text)
        }
        PimdirQueryPredicate::Address { role, text } => {
            holds(role.map_or("people", |role| role.as_str()), text)
        }
        PimdirQueryPredicate::Flag(flag) => mail.flags.contains(flag),
        PimdirQueryPredicate::Encrypted => mail.extract.status == PimdirExtractStatus::Encrypted,
        PimdirQueryPredicate::HasBody => mail.body,
        PimdirQueryPredicate::HasAttachment => mail.summary.attachment == Some(true),
        PimdirQueryPredicate::Kind(kind) => *kind == PimdirQueryKind::Mail,
        PimdirQueryPredicate::Mid(id) => mail.summary.message_id.as_ref() == Some(id),
        PimdirQueryPredicate::Date(range) => {
            let (since, until) = range.bounds(now, zone);
            let key = mail.sort_key.as_str();
            !key.is_empty()
                && since.is_none_or(|since| key >= time::rfc3339(since).as_str())
                && until.is_none_or(|until| key < time::rfc3339(until).as_str())
        }
        PimdirQueryPredicate::HasProperty(_)
        | PimdirQueryPredicate::Status(_)
        | PimdirQueryPredicate::Priority(_)
        | PimdirQueryPredicate::Category(_) => false,
        PimdirQueryPredicate::Person(_)
        | PimdirQueryPredicate::Account(_)
        | PimdirQueryPredicate::Collection(_)
        | PimdirQueryPredicate::Changed { .. }
        | PimdirQueryPredicate::Thread(_)
        | PimdirQueryPredicate::Id(_)
        | PimdirQueryPredicate::Linked { .. }
        | PimdirQueryPredicate::Label(_)
        | PimdirQueryPredicate::Property { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use alloc::{string::ToString, vec};

    use crate::search::{mail::*, query::PimdirQuery};

    const HEADER: &[u8] = b"Message-ID: <a@x>\r\nFrom: Jane <Jane@Example.org>\r\n\
        Subject: Budget\r\nDate: Mon, 10 Aug 2026 08:00:00 +0000\r\n\r\n";

    /// A message read from its header alone holds no body, and answers its
    /// summary's fields, addresses and flags.
    #[test]
    fn a_header_alone_answers_its_summary() {
        let mails = [PimdirMail::read_meta(
            HEADER,
            Some(10),
            None,
            vec!["\\Seen".to_string()],
        )];
        let matched = |query: &str| {
            PimdirQuery::parse(query)
                .unwrap()
                .match_mails(&mails, 0, &TimeZone::UTC)
                .unwrap()
        };
        assert_eq!(matched("budget from:jane@example.org"), vec![true]);
        assert_eq!(
            matched("mid:<a@x> is:read date:2026-08-10T08:00Z"),
            vec![true]
        );
        assert_eq!(matched("has:body"), vec![false]);
        assert_eq!(matched("is:unread"), vec![false]);
    }

    /// A term no message answers alone refuses the match, naming its span.
    #[test]
    fn a_term_no_message_answers_is_refused() {
        let query = PimdirQuery::parse("budget person:jane").unwrap();
        let mails = [PimdirMail::read_meta(HEADER, None, None, vec![])];
        assert_eq!(
            query.match_mails(&mails, 0, &TimeZone::UTC),
            Err(PimdirMailMatchError { start: 7, end: 18 })
        );
    }
}
