//! # Cards
//!
//! A query (SEARCH §8) answered over cards held in memory, with no index
//! and no store: what a client listing a server's address books matches
//! each card by, so it answers the hit set an index holding the same
//! cards answers (SEARCH §11's contact cases).
//!
//! A card is read as the indexer reads it: its fields extracted (SEARCH
//! §6), each folded (§5) and split into tokens as FTS5's `unicode61`
//! splits them, letters, numbers and private-use characters forming
//! tokens and every other character separating them. Its addresses are
//! the store's rows of it (STORAGE Annex A.6). A term reading what the
//! store alone holds, a flag, an account, a collection, a change stamp,
//! a thread, an item or its references, is answered by no card:
//! [`PimdirQuery::store_terms`] names them, and a match meeting one is
//! refused.
//!
//! Outside the hit-set guarantee, as SEARCH §2 puts the indexer's own
//! reading: a character a Unicode version classifies otherwise than the
//! tokenizer's tables, and a letter whose lowercase SQLite folds
//! otherwise than Rust.

use alloc::{string::String, vec::Vec};
use core::fmt;

use crate::{
    search::{
        extract::{self, PimdirExtract, PimdirExtractStatus},
        fold,
        query::{
            PimdirQuery, PimdirQueryKind, PimdirQueryPredicate, PimdirQueryProperty,
            PimdirQueryTerm, PimdirQueryText, PimdirQueryTextField,
        },
    },
    summary::{self, PimdirAddressRole, PimdirSummary},
};

/// One card as a query reads it: its fields as the indexer extracts them,
/// and its addresses as the store keeps them.
#[derive(Clone, Debug)]
pub struct PimdirCard {
    extract: PimdirExtract,
    emails: Vec<String>,
}

impl PimdirCard {
    /// Reads a `text/vcard` body as the indexer does (SEARCH §6): one no
    /// parser accepts is read from its summary, its name and addresses.
    pub fn read(body: &[u8]) -> Self {
        let derived = summary::contact::derive(body);
        let addresses: Vec<_> = derived
            .summary
            .as_ref()
            .map(|summary| {
                summary
                    .addresses()
                    .into_iter()
                    .map(|(role, address)| (role, address.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let emails = addresses
            .iter()
            .filter(|(role, _)| *role == PimdirAddressRole::Email)
            .map(|(_, address)| address.address.clone())
            .collect();

        let mut extract = PimdirExtract::contact(body);
        if extract.status == PimdirExtractStatus::Unparseable {
            let summary: Option<&PimdirSummary> = derived.summary.as_ref();
            extract = PimdirExtract::summary(summary, &addresses);
        }

        Self { extract, emails }
    }

    /// Whether the column `name` holds the tokens of `text` consecutively,
    /// its last one a prefix when the text is (SEARCH §8.5).
    fn holds(&self, name: &str, text: &PimdirQueryText) -> bool {
        fold::holds(self.extract.column(name), text)
    }
}

/// A term no card answers, which only a store holds the answer to
/// (`tag:`, `flag:`, `is:` but `is:encrypted`, `account:`,
/// `collection:`, `folder:`, `changed:`, `thread:`, `id:`, `linked:`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirCardMatchError {
    /// The term's first offset, in Unicode scalar values.
    pub start: usize,
    /// The offset after its last.
    pub end: usize,
}

impl fmt::Display for PimdirCardMatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Query term at {}..{} reads the store, which no card answers",
            self.start, self.end
        )
    }
}

impl core::error::Error for PimdirCardMatchError {}
impl PimdirQuery {
    /// The terms a store alone answers, in reading order: a client holding
    /// cards and no store refuses a query naming one before it lists any.
    pub fn store_terms(&self) -> Vec<&PimdirQueryTerm> {
        self.terms()
            .into_iter()
            .map(|(term, _)| term)
            .filter(|term| reads_store(&term.predicate))
            .collect()
    }

    /// Whether each of `cards` is a hit of the query, in order (SEARCH §8),
    /// as an index holding those cards alone answers: `person:` reads them
    /// all, a card naming an address a card it matches by title holds
    /// being one. A term a store alone answers refuses the match.
    pub fn match_cards(&self, cards: &[PimdirCard]) -> Result<Vec<bool>, PimdirCardMatchError> {
        if let Some(term) = self.store_terms().first() {
            return Err(PimdirCardMatchError {
                start: term.start,
                end: term.end,
            });
        }

        let dated = self.terms().iter().any(|(term, _)| {
            matches!(
                term.predicate,
                PimdirQueryPredicate::Kind(PimdirQueryKind::Contact)
            )
        });
        let matcher = Matcher { cards, dated };

        Ok((0..cards.len()).map(|at| matcher.node(self, at)).collect())
    }
}

/// Whether a predicate reads what a store alone holds.
fn reads_store(predicate: &PimdirQueryPredicate) -> bool {
    matches!(
        predicate,
        PimdirQueryPredicate::Flag(_)
            | PimdirQueryPredicate::Account(_)
            | PimdirQueryPredicate::Collection(_)
            | PimdirQueryPredicate::Changed { .. }
            | PimdirQueryPredicate::Thread(_)
            | PimdirQueryPredicate::Id(_)
            | PimdirQueryPredicate::Linked { .. }
    )
}

/// One match over a set of cards.
struct Matcher<'a> {
    cards: &'a [PimdirCard],
    /// Whether a `kind:contact` term stands anywhere in the query, which
    /// lets a date term through to a card (SEARCH §8.1).
    dated: bool,
}

impl Matcher<'_> {
    fn node(&self, node: &PimdirQuery, at: usize) -> bool {
        match node {
            PimdirQuery::And(operands) => operands.iter().all(|operand| self.node(operand, at)),
            PimdirQuery::Or(operands) => operands.iter().any(|operand| self.node(operand, at)),
            PimdirQuery::Not(operand) => !self.node(operand, at),
            PimdirQuery::Group(operand) => self.node(operand, at),
            PimdirQuery::Term(term) => self.term(&term.predicate, at),
        }
    }

    fn term(&self, predicate: &PimdirQueryPredicate, at: usize) -> bool {
        let card = &self.cards[at];
        match predicate {
            PimdirQueryPredicate::Text { field, text } => match field {
                PimdirQueryTextField::Any => {
                    extract::COLUMNS.iter().any(|name| card.holds(name, text))
                }
                PimdirQueryTextField::Title => card.holds("title", text),
                PimdirQueryTextField::Body => card.holds("body", text),
                PimdirQueryTextField::Attachment => card.holds("attachment", text),
                PimdirQueryTextField::Place => card.holds("place", text),
            },
            PimdirQueryPredicate::Address { role, text } if text.text.contains('@') => {
                if role.is_some_and(|role| role != PimdirAddressRole::Email) {
                    return false;
                }
                let address = text.text.to_lowercase();
                card.emails
                    .iter()
                    .any(|held| match address.strip_prefix('@') {
                        Some(domain) => held
                            .rsplit_once('@')
                            .is_some_and(|(_, at)| at.to_lowercase() == domain),
                        None => *held == address,
                    })
            }
            PimdirQueryPredicate::Address { role, text } => {
                card.holds(role.map_or("people", |role| role.as_str()), text)
            }
            PimdirQueryPredicate::Person(text) => self.cards.iter().any(|named| {
                named.holds("title", text)
                    && (core::ptr::eq(named, card)
                        || named.emails.iter().any(|email| card.emails.contains(email)))
            }),
            PimdirQueryPredicate::HasBody => true,
            PimdirQueryPredicate::HasProperty(property) => match property {
                PimdirQueryProperty::Phone => card.extract.tel,
                PimdirQueryProperty::Address => card.extract.adr,
                PimdirQueryProperty::Birthday => card.extract.bday,
                PimdirQueryProperty::Photo => card.extract.photo,
            },
            PimdirQueryPredicate::Kind(kind) => *kind == PimdirQueryKind::Contact,
            PimdirQueryPredicate::Category(category) => {
                card.extract.categories.contains(&fold::value(category))
            }
            PimdirQueryPredicate::Date(_) => self.dated,
            PimdirQueryPredicate::Encrypted
            | PimdirQueryPredicate::HasAttachment
            | PimdirQueryPredicate::Mid(_)
            | PimdirQueryPredicate::Status(_)
            | PimdirQueryPredicate::Priority(_) => false,
            PimdirQueryPredicate::Flag(_)
            | PimdirQueryPredicate::Account(_)
            | PimdirQueryPredicate::Collection(_)
            | PimdirQueryPredicate::Changed { .. }
            | PimdirQueryPredicate::Thread(_)
            | PimdirQueryPredicate::Id(_)
            | PimdirQueryPredicate::Linked { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use crate::search::{card::*, query::PimdirQuery};

    const CARD: &str = "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Zoë Durand\r\n\
        EMAIL:Zoe.Durand@Example.ORG\r\nTEL:+33 1 23 45 67 89\r\n\
        CATEGORIES:Family,flat\r\nEND:VCARD\r\n";

    const OTHER: &str = "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Desk\r\n\
        EMAIL:zoe.durand@example.org\r\nEND:VCARD\r\n";

    fn matched(query: &str) -> Vec<bool> {
        let cards = [
            PimdirCard::read(CARD.as_bytes()),
            PimdirCard::read(OTHER.as_bytes()),
        ];
        PimdirQuery::parse(query)
            .unwrap()
            .match_cards(&cards)
            .unwrap()
    }

    /// Text folds and splits as the index does: a diacritic, a prefix, a
    /// phrase whose words follow each other, and a telephone's digits.
    #[test]
    fn a_card_matches_text_as_the_index_reads_it() {
        assert_eq!(matched("zoe"), vec![true, true], "the address's local part");
        assert_eq!(matched("title:zoe"), vec![true, false]);
        assert_eq!(matched("title:\"zoe durand\""), vec![true, false]);
        assert_eq!(matched("title:\"durand zoe\""), vec![false, false]);
        assert_eq!(matched("title:dur*"), vec![true, false]);
        assert_eq!(matched("33123456789"), vec![true, false]);
    }

    /// An address is sought whole or by its domain, lowercased; a card
    /// holds the email role alone.
    #[test]
    fn a_card_is_sought_by_its_addresses() {
        assert_eq!(matched("email:ZOE.DURAND@example.org"), vec![true, true]);
        assert_eq!(matched("with:@EXAMPLE.org"), vec![true, true]);
        assert_eq!(matched("from:zoe.durand@example.org"), vec![false, false]);
    }

    /// `person:` reads every card held: one sharing an address with a card
    /// its name matches is a hit too.
    #[test]
    fn a_person_reaches_the_cards_sharing_an_address() {
        assert_eq!(matched("person:zoe"), vec![true, true]);
        assert_eq!(matched("person:desk"), vec![true, true]);
    }

    /// The marks, the categories and the kind answer from the card; a date
    /// reaches it only beside `kind:contact`.
    #[test]
    fn a_card_answers_its_marks_and_its_kind() {
        assert_eq!(matched("has:phone"), vec![true, false]);
        assert_eq!(matched("category:FAMILY"), vec![true, false]);
        assert_eq!(matched("kind:contact -kind:mail"), vec![true, true]);
        assert_eq!(matched("date:2026"), vec![false, false]);
        assert_eq!(matched("date:2026 or kind:contact"), vec![true, true]);
    }

    /// A term only a store answers refuses the match, naming its span.
    #[test]
    fn a_store_term_is_refused() {
        let query = PimdirQuery::parse("zoe tag:x").unwrap();
        assert_eq!(query.store_terms().len(), 1);
        let cards = [PimdirCard::read(CARD.as_bytes())];
        assert_eq!(
            query.match_cards(&cards),
            Err(PimdirCardMatchError { start: 4, end: 9 })
        );
    }
}
