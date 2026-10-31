//! # Threads
//!
//! Mail threads derived for the index (SEARCH §9): one `message` row per
//! message id a held message bears or names as its parent, present in the
//! store or not, its `parent` the first `In-Reply-To` id, else the last
//! `References` id, read from the body the index holds, else from the
//! summary's `mail_in_reply_to`. A thread is a connected set over those
//! rows, named after its present member with the lowest
//! `(sort_key, link_id)`.
//!
//! A refresh collects the message ids its batch touched (a mail placement
//! new, rebodied, rekeyed or dropped) and relinks them before the batch
//! commits: their parents re-read, the edge closing a cycle ignored, and
//! every thread they reach recomputed. No joining by subject.
//!
//! The relink here reads nothing: it runs over the messages the refresh
//! has read ([`PimdirIndexMessage`]), changes them in memory, and answers
//! the ops writing the result, or the messages and bodies it still needs
//! read ([`Need`]), the refresh asking for them and running it again.

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::{String, ToString},
    vec,
    vec::Vec,
};

use log::trace;
use mail_parser::MessageParser;

use crate::{
    object::PimdirHash,
    search::refresh::{PimdirIndexMessage, PimdirIndexMessageRow, PimdirIndexOp},
};

/// The parent a held body names, by hash (SEARCH §9): `Some` with what
/// its headers name, possibly nothing, `None` for a body that could not be
/// read or whose headers do not parse, its summary's then.
pub(crate) type Heads = BTreeMap<PimdirHash, Option<Option<String>>>;

/// What a relink still needs read before it can answer.
#[derive(Debug, Default)]
pub(crate) struct Need {
    /// The message ids not read yet.
    pub(crate) messages: BTreeSet<String>,
    /// The bodies whose headers are not read yet.
    pub(crate) bodies: BTreeSet<PimdirHash>,
}

impl Need {
    fn is_empty(&self) -> bool {
        self.messages.is_empty() && self.bodies.is_empty()
    }
}

/// The parent a body's headers name (SEARCH §9): its first `In-Reply-To`
/// id, else its last `References` id; `None` when the headers do not
/// parse.
pub(crate) fn parent(body: &[u8]) -> Option<Option<String>> {
    let message = MessageParser::default().parse_headers(body)?;
    let first = message
        .in_reply_to()
        .as_text_list()
        .and_then(|ids| ids.first())
        .or_else(|| message.references().as_text_list()?.last());
    Some(first.map(|id| id.to_string()).filter(|id| !id.is_empty()))
}

/// Rereads the parents of the message ids a batch touched and recomputes
/// every thread they reach (SEARCH §9), over the messages read so far:
/// the ops writing the result, the thread rows first, then the message
/// rows, then the thread rows no message points to any more, so a
/// consumer enforcing `message.thread`'s reference never sees a message
/// point to a row not written yet. A message or a body not read yet is
/// the [`Need`] answered instead.
pub(crate) fn relink(
    nodes: &BTreeMap<String, PimdirIndexMessage>,
    heads: &Heads,
    touched: &BTreeSet<String>,
) -> Result<Vec<PimdirIndexOp>, Need> {
    let mut graph = Graph {
        nodes,
        heads,
        current: BTreeMap::new(),
        need: Need::default(),
    };

    let mut seeds = BTreeSet::new();
    for link_id in touched {
        let Some(node) = graph.node(link_id) else {
            continue;
        };
        let held = node.row.clone();
        let held = graph.current.get(link_id).cloned().unwrap_or(held);
        let present = node.presence.is_some();
        if held.is_none() && !present {
            continue;
        }

        let parent = match present {
            true => match graph.parent(node) {
                Some(parent) => parent,
                None => continue,
            },
            false => None,
        };
        // NOTE: the edge that would close a cycle is ignored, a message
        // naming itself included.
        let parent = match parent {
            Some(parent) => match graph.reaches(&parent, link_id) {
                Some(true) => {
                    trace!("ignoring {link_id} -> {parent}, which closes a cycle");
                    None
                }
                Some(false) => Some(parent),
                None => continue,
            },
            None => None,
        };
        trace!("message {link_id} replies to {parent:?}");

        match held {
            None => graph.set(
                link_id,
                Some(PimdirIndexMessageRow {
                    thread: link_id.clone(),
                    parent: parent.clone(),
                }),
            ),
            Some(row) if row.parent != parent => {
                graph.set(
                    link_id,
                    Some(PimdirIndexMessageRow {
                        thread: row.thread,
                        parent: parent.clone(),
                    }),
                );
                seeds.extend(row.parent);
            }
            Some(_) => (),
        }
        if let Some(parent) = &parent {
            match graph.row(parent) {
                None => continue,
                Some(None) => graph.set(
                    parent,
                    Some(PimdirIndexMessageRow {
                        thread: parent.clone(),
                        parent: None,
                    }),
                ),
                Some(Some(_)) => (),
            }
        }
        seeds.insert(link_id.clone());
    }

    if !graph.need.is_empty() {
        return Err(graph.need);
    }
    graph.rethread(&seeds)
}

/// The messages a relink reads, the rows it changed over them, and what
/// it found missing.
struct Graph<'a> {
    nodes: &'a BTreeMap<String, PimdirIndexMessage>,
    heads: &'a Heads,
    /// The rows changed so far, `None` for a row dropped.
    current: BTreeMap<String, Option<PimdirIndexMessageRow>>,
    need: Need,
}

impl<'a> Graph<'a> {
    /// The message of an id as read, `None` when not read yet.
    fn node(&mut self, link_id: &str) -> Option<&'a PimdirIndexMessage> {
        let nodes = self.nodes;
        let node = nodes.get(link_id);
        if node.is_none() {
            self.need.messages.insert(link_id.to_string());
        }
        node
    }

    /// The row of an id as changed so far: `Some(None)` for an id with no
    /// row, `None` when not read yet.
    fn row(&mut self, link_id: &str) -> Option<Option<PimdirIndexMessageRow>> {
        if let Some(row) = self.current.get(link_id) {
            return Some(row.clone());
        }
        Some(self.node(link_id)?.row.clone())
    }

    fn set(&mut self, link_id: &str, row: Option<PimdirIndexMessageRow>) {
        self.current.insert(link_id.to_string(), row);
    }

    /// The parent of a held message (SEARCH §9): from a body the index
    /// holds, what its headers name; from its summary's
    /// `mail_in_reply_to` when it holds none, or none it can read. `None`
    /// when the body is not read yet.
    fn parent(&mut self, node: &PimdirIndexMessage) -> Option<Option<String>> {
        if let Some(hash) = &node.hash {
            match self.heads.get(hash) {
                None => {
                    self.need.bodies.insert(hash.clone());
                    return None;
                }
                Some(Some(parent)) => return Some(parent.clone()),
                Some(None) => (),
            }
        }
        Some(node.in_reply_to.clone())
    }

    /// Whether following parents up from `from` reaches `to`, `from`
    /// itself included: the edge `to -> from` would close a cycle. `None`
    /// when a message on the way is not read yet.
    fn reaches(&mut self, from: &str, to: &str) -> Option<bool> {
        let mut seen = BTreeSet::new();
        let mut at = Some(from.to_string());
        while let Some(link_id) = at {
            if link_id == to {
                return Some(true);
            }
            if !seen.insert(link_id.clone()) {
                return Some(false);
            }
            at = self.row(&link_id)?.and_then(|row| row.parent);
        }
        Some(false)
    }

    /// The ids naming a read one as their parent, as changed so far.
    fn children(&self, link_id: &str) -> Vec<String> {
        let mut children: Vec<String> = self.nodes[link_id]
            .children
            .iter()
            .filter(|child| match self.current.get(*child) {
                Some(Some(row)) => row.parent.as_deref() == Some(link_id),
                Some(None) => false,
                None => true,
            })
            .cloned()
            .collect();
        for (child, row) in &self.current {
            if let Some(row) = row
                && row.parent.as_deref() == Some(link_id)
                && !children.contains(child)
            {
                children.push(child.clone());
            }
        }
        children
    }

    /// Recomputes the thread of every component a seed is in: a component
    /// no placement holds a member of dropped whole (a message row nobody
    /// holds has no parent, so one nobody names is alone), else every
    /// member pointed to one thread row named after the present member
    /// with the lowest `(sort_key, link_id)`; last the thread rows no
    /// member points to any more.
    fn rethread(mut self, seeds: &BTreeSet<String>) -> Result<Vec<PimdirIndexOp>, Need> {
        let mut done = BTreeSet::new();
        let mut stale = BTreeSet::new();
        let mut ops = Vec::new();

        for seed in seeds {
            if done.contains(seed) {
                continue;
            }

            let mut members: BTreeMap<String, PimdirIndexMessageRow> = BTreeMap::new();
            let mut whole = true;
            let mut queue = vec![seed.clone()];
            while let Some(link_id) = queue.pop() {
                if members.contains_key(&link_id) {
                    continue;
                }
                let Some(row) = self.row(&link_id) else {
                    whole = false;
                    continue;
                };
                let Some(row) = row else {
                    continue;
                };
                queue.extend(row.parent.clone());
                queue.extend(self.children(&link_id));
                members.insert(link_id, row);
            }
            if !whole {
                continue;
            }
            done.extend(members.keys().cloned());
            stale.extend(members.values().map(|row| row.thread.clone()));

            let present: Vec<(&String, _)> = members
                .keys()
                .filter_map(|link_id| Some((link_id, self.nodes[link_id].presence.as_ref()?)))
                .collect();
            let Some((id, _)) = present
                .iter()
                .min_by(|(a, x), (b, y)| (&x.sort_key, a).cmp(&(&y.sort_key, b)))
            else {
                trace!("thread of {seed} holds no message, dropping it");
                for link_id in members.keys() {
                    self.current.insert(link_id.clone(), None);
                }
                continue;
            };
            let id = (*id).clone();
            let first = present
                .iter()
                .filter_map(|(_, presence)| presence.first.clone())
                .min();
            let last = present
                .iter()
                .filter_map(|(_, presence)| presence.last.clone())
                .max();
            trace!("thread {id} holds {} message(s)", present.len());

            ops.push(PimdirIndexOp::PutThread {
                id: id.clone(),
                first: first.unwrap_or_default(),
                last: last.unwrap_or_default(),
                count: present.len() as u64,
            });
            for (link_id, row) in members {
                if row.thread != id {
                    self.current.insert(
                        link_id,
                        Some(PimdirIndexMessageRow {
                            thread: id.clone(),
                            parent: row.parent,
                        }),
                    );
                }
            }
        }

        if !self.need.is_empty() {
            return Err(self.need);
        }

        for (link_id, row) in &self.current {
            let read = self.nodes.get(link_id).and_then(|node| node.row.as_ref());
            match row {
                Some(row) if read != Some(row) => ops.push(PimdirIndexOp::PutMessage {
                    link_id: link_id.clone(),
                    thread: row.thread.clone(),
                    parent: row.parent.clone(),
                }),
                None if read.is_some() => ops.push(PimdirIndexOp::DropMessage(link_id.clone())),
                _ => (),
            }
        }
        ops.extend(stale.into_iter().map(PimdirIndexOp::DropThread));
        Ok(ops)
    }
}

#[cfg(test)]
mod tests {
    use alloc::{
        collections::{BTreeMap, BTreeSet},
        string::{String, ToString},
        vec,
    };

    use crate::search::{
        refresh::{PimdirIndexMessage, PimdirIndexOp, PimdirIndexPresence},
        thread::*,
    };

    fn present(sort_key: &str, in_reply_to: Option<&str>) -> PimdirIndexMessage {
        PimdirIndexMessage {
            presence: Some(PimdirIndexPresence {
                sort_key: sort_key.into(),
                first: Some(sort_key.into()),
                last: Some(sort_key.into()),
            }),
            in_reply_to: in_reply_to.map(String::from),
            ..Default::default()
        }
    }

    /// Two replies to a parent nobody holds are one thread, named after the
    /// earlier reply, the parent's row written though it is absent
    /// (SEARCH §9); the relink first asks for the parent it has not read.
    #[test]
    fn two_replies_to_an_absent_parent_are_one_thread() {
        let mut nodes = BTreeMap::from([
            ("b".to_string(), present("2026-02", Some("a"))),
            ("c".to_string(), present("2026-01", Some("a"))),
        ]);
        let touched = BTreeSet::from(["b".to_string(), "c".to_string()]);

        let Err(need) = relink(&nodes, &Heads::new(), &touched) else {
            panic!("the parent is not read yet");
        };
        assert_eq!(need.messages, BTreeSet::from(["a".to_string()]));
        nodes.insert("a".into(), PimdirIndexMessage::default());

        let ops = relink(&nodes, &Heads::new(), &touched).unwrap();
        assert_eq!(
            ops[0],
            PimdirIndexOp::PutThread {
                id: "c".into(),
                first: "2026-01".into(),
                last: "2026-02".into(),
                count: 2,
            }
        );
        let messages: BTreeSet<(String, String)> = ops
            .iter()
            .filter_map(|op| match op {
                PimdirIndexOp::PutMessage {
                    link_id, thread, ..
                } => Some((link_id.clone(), thread.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            messages,
            vec!["a", "b", "c"]
                .into_iter()
                .map(|id| (id.to_string(), "c".to_string()))
                .collect()
        );
    }

    /// The edge that would close a cycle is ignored, a message naming
    /// itself included.
    #[test]
    fn a_message_naming_itself_stands_alone() {
        let nodes = BTreeMap::from([("a".to_string(), present("2026-01", Some("a")))]);
        let touched = BTreeSet::from(["a".to_string()]);
        let ops = relink(&nodes, &Heads::new(), &touched).unwrap();
        assert!(ops.contains(&PimdirIndexOp::PutMessage {
            link_id: "a".into(),
            thread: "a".into(),
            parent: None,
        }));
    }
}
