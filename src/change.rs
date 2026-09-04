//! # Changes and writes
//!
//! The outbound [`PimdirChange`] the engine asks the consumer to push to
//! the remote, and the inbound [`PimdirWriteOp`] it asks it to persist.
//!
//! A move is two halves, a create in the target and a remove of the
//! source, each derived by its own collection's sync in either order.
//! Both can deliver the item, so the remove carries the link id its
//! destination receives and a consumer relocates only while it is absent.
//!
//! Neither half may be dropped for the other: the remove keeps a move safe
//! when the target syncs last, and the create keeps it working through a
//! [hub](crate::hub), whose bindings carry no origin. A create whose
//! origin is already relocated is rejected and stays visibly pending.

use alloc::{format, string::String, vec::Vec};

use crate::{
    collection::{PimdirCheckpoint, PimdirCollectionId},
    object::{PimdirHash, PimdirObject},
    placement::{PimdirFlags, PimdirHandle, PimdirLinkId, PimdirOrigin, PimdirPlacement},
};

/// A change to push to the remote: what to do, and the key naming it.
///
/// Pushes are at-least-once: a crash between a serviced push and its
/// recording write replays the change, within one
/// [`PUSH_CHUNK`](crate::sync::PimdirSync::PUSH_CHUNK), keyed the same.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirChange {
    /// What the remote is asked to do.
    pub kind: PimdirChangeKind,
    /// The idempotency key naming this change, derived by [`new`](Self::new).
    pub key: PimdirChangeKey,
}

impl PimdirChange {
    /// Keys `kind` in `collection`, the only way a change is made.
    pub fn new(collection: &PimdirCollectionId, kind: PimdirChangeKind) -> Self {
        let key = kind.key(collection);

        Self { kind, key }
    }

    /// The member this change acts on.
    pub fn handle(&self) -> &PimdirHandle {
        match &self.kind {
            PimdirChangeKind::Add { handle, .. } => handle,
            PimdirChangeKind::Remove { handle, .. } => handle,
            PimdirChangeKind::SetFlags { handle, .. } => handle,
            PimdirChangeKind::Update { handle, .. } => handle,
        }
    }
}

/// What a [`PimdirChange`] asks the remote to do.
///
/// Membership is add or remove only. An add reuses a server-side copy or
/// move when it carries an [`PimdirOrigin`], else it uploads the stored
/// body. How a move splits into both is in the module header.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirChangeKind {
    /// Add a member under a provisional `handle` the push reconciles.
    ///
    /// The server-assigned handle comes back in the
    /// [push result](crate::remote::PimdirPushResult::assigned).
    Add {
        /// The provisional handle the member is staged under locally.
        handle: PimdirHandle,
        /// The item identity when resolved, telling a retried add it landed.
        link_id: Option<PimdirLinkId>,
        /// The flag set to create the member with.
        ///
        /// A server-side copy may inherit the source flags instead, the
        /// skew reconciling on the next sync.
        flags: PimdirFlags,
        /// Where the body already lives, `None` for an append of `object`.
        ///
        /// When the origin is gone a consumer holding `object` may upload
        /// it instead; without a body, rejecting keeps the create pending.
        origin: Option<PimdirOrigin>,
        /// The stored body to upload when there is no `origin`.
        object: Option<PimdirHash>,
    },
    /// Remove a member, moving it into `to` when set.
    ///
    /// The disposal of a plain delete (expunge, trash) is the consumer's
    /// policy.
    Remove {
        /// The member to remove.
        handle: PimdirHandle,
        /// The move destination, or `None` for a delete.
        to: Option<PimdirCollectionId>,
        /// The item identity when resolved, the delivery key of a move.
        ///
        /// A `to` already holding it was served by the move's other half,
        /// so the remove is a plain delete.
        link_id: Option<PimdirLinkId>,
        /// The last-synced revision as a precondition (a WebDAV If-Match).
        if_match: Option<String>,
    },
    /// Replace a member's flag set.
    SetFlags {
        /// The member to update.
        handle: PimdirHandle,
        /// The new flag set.
        flags: PimdirFlags,
    },
    /// Replace a member's content in place with a locally edited body.
    Update {
        /// The member to update.
        handle: PimdirHandle,
        /// The hash of the new body in the object store.
        object: PimdirHash,
        /// The last-synced revision as a precondition (a WebDAV If-Match).
        if_match: Option<String>,
    },
}

impl PimdirChangeKind {
    /// Derives this kind's idempotency key in `collection`.
    ///
    /// The key covers the collection, the handle, the kind and the target
    /// state the change makes true. A precondition is not part of it: a
    /// retry of one operation is one operation.
    fn key(&self, collection: &PimdirCollectionId) -> PimdirChangeKey {
        let handle = match self {
            Self::Add { handle, .. } => handle,
            Self::Remove { handle, .. } => handle,
            Self::SetFlags { handle, .. } => handle,
            Self::Update { handle, .. } => handle,
        };

        let mut digest = PimdirChangeDigest::new();

        digest
            .field(collection.as_str().as_bytes())
            .field(handle.as_str().as_bytes());

        match self {
            Self::Add {
                link_id,
                flags,
                origin,
                object,
                ..
            } => {
                digest
                    .field(b"add")
                    .option(link_id.as_ref().map(|link| link.as_str().as_bytes()))
                    .flags(flags);
                match origin {
                    Some(origin) => digest
                        .field(b"1")
                        .field(origin.collection.as_str().as_bytes())
                        .field(origin.handle.as_str().as_bytes()),
                    None => digest.field(b"0"),
                }
                .option(object.as_ref().map(|hash| hash.as_str().as_bytes()));
            }
            Self::Remove { to, .. } => {
                digest
                    .field(b"remove")
                    .option(to.as_ref().map(|to| to.as_str().as_bytes()));
            }
            Self::SetFlags { flags, .. } => {
                digest.field(b"set-flags").flags(flags);
            }
            Self::Update { object, .. } => {
                digest.field(b"update").field(object.as_str().as_bytes());
            }
        }

        digest.finish()
    }
}

crate::pimdir_id! {
    /// The idempotency key naming a derived change.
    ///
    /// Sixteen lowercase hexadecimal characters, opaque to the engine. A
    /// consumer records the keys it applied and recognises a replay.
    PimdirChangeKey, Ord, PartialOrd, Hash,
}

/// The digest a [`PimdirChangeKind`] is folded into to key it.
///
/// FNV-1a, sixty-four bits: an idempotency key needs determinism, not
/// resistance to a forged collision.
struct PimdirChangeDigest(u64);

impl PimdirChangeDigest {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    fn new() -> Self {
        Self(Self::OFFSET)
    }

    /// Folds one field in, terminated so two splits of one byte string differ.
    fn field(&mut self, bytes: &[u8]) -> &mut Self {
        for byte in bytes.iter().chain(&[0]) {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }

        self
    }

    /// Folds an optional field in, presence keying on its own.
    fn option(&mut self, bytes: Option<&[u8]>) -> &mut Self {
        match bytes {
            Some(bytes) => self.field(b"1").field(bytes),
            None => self.field(b"0"),
        }
    }

    /// Folds a flag set in, length first, an unknown set apart from an empty.
    fn flags(&mut self, flags: &PimdirFlags) -> &mut Self {
        let Some(flags) = flags.known() else {
            return self.field(b"unknown");
        };

        self.field(b"known").field(&flags.len().to_le_bytes());
        for flag in flags {
            self.field(flag.as_bytes());
        }

        self
    }

    fn finish(&self) -> PimdirChangeKey {
        PimdirChangeKey::from(format!("{:016x}", self.0))
    }
}

/// Why a placement is being dropped.
///
/// A storage sharing one item across sources (a [hub](crate::hub)) has to
/// tell the two apart, or a housekeeping drop becomes a delete elsewhere.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirDropReason {
    /// The item is gone, a confirmed delete or a vanished member.
    Deleted,
    /// Only this row is gone: a provisional handle an accepted add replaced.
    Superseded,
    /// Only this row is gone: a handle a rebuild renumbered (SYNC §8), which
    /// is also the storage's signal to bump the collection's generation.
    Rekeyed,
}

/// A write to persist in local storage, applied atomically per batch.
///
/// A placement references an object once per pointing field (its own
/// `object` and its base's). The consumer counts references by diffing an
/// upsert against the row it replaces, and may collect an unreferenced one.
// NOTE: upserts dominate every batch, so boxing the placement would only
// add indirection on the hot variant.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirWriteOp {
    /// Insert or replace a placement.
    UpsertPlacement(PimdirPlacement),
    /// Drop a placement.
    DropPlacement {
        /// The owning collection.
        collection: PimdirCollectionId,
        /// The handle to drop.
        handle: PimdirHandle,
        /// Whether the item itself is gone, or only this row of it.
        reason: PimdirDropReason,
    },
    /// Store an object body.
    ///
    /// Storing takes no reference: the pointing upsert may ride in a later
    /// batch, so a storage must not collect the object at the commit.
    StoreObject {
        /// The object metadata.
        object: PimdirObject,
        /// The body bytes, `None` when the consumer persisted them already.
        body: Option<Vec<u8>>,
    },
    /// Set a collection's sync checkpoint.
    SetCheckpoint {
        /// The collection to checkpoint.
        collection: PimdirCollectionId,
        /// The new checkpoint.
        checkpoint: PimdirCheckpoint,
    },
}

#[cfg(test)]
mod tests {
    use alloc::{collections::BTreeSet, string::String, vec, vec::Vec};

    use crate::{
        change::{PimdirChange, PimdirChangeKey, PimdirChangeKind},
        collection::PimdirCollectionId,
        object::PimdirHash,
        placement::{PimdirFlags, PimdirHandle, PimdirLinkId, PimdirOrigin},
    };

    fn key(collection: &PimdirCollectionId, kind: PimdirChangeKind) -> PimdirChangeKey {
        PimdirChange::new(collection, kind).key
    }

    fn set_flags(handle: &str, flags: &[&str]) -> PimdirChangeKind {
        PimdirChangeKind::SetFlags {
            handle: PimdirHandle::from(handle),
            flags: PimdirFlags::from_iter(flags.iter().copied()),
        }
    }

    fn update(handle: &str, object: &str, if_match: Option<&str>) -> PimdirChangeKind {
        PimdirChangeKind::Update {
            handle: PimdirHandle::from(handle),
            object: PimdirHash::from(object),
            if_match: if_match.map(String::from),
        }
    }

    #[test]
    fn the_same_derived_change_keys_the_same() {
        let inbox = PimdirCollectionId::from("inbox");

        assert_eq!(
            key(&inbox, set_flags("1", &["seen"])),
            key(&inbox, set_flags("1", &["seen"])),
        );
    }

    #[test]
    fn a_key_separates_collection_handle_kind_and_target_state() {
        let inbox = PimdirCollectionId::from("inbox");
        let archive = PimdirCollectionId::from("archive");

        let keys: Vec<PimdirChangeKey> = vec![
            key(&inbox, set_flags("1", &["seen"])),
            key(&archive, set_flags("1", &["seen"])),
            key(&inbox, set_flags("2", &["seen"])),
            key(&inbox, set_flags("1", &["flagged"])),
            key(&inbox, set_flags("1", &["seen", "flagged"])),
            key(&inbox, set_flags("1", &[])),
            key(&inbox, update("1", "aaa", None)),
            key(&inbox, update("1", "bbb", None)),
            key(
                &inbox,
                PimdirChangeKind::Remove {
                    handle: PimdirHandle::from("1"),
                    to: None,
                    link_id: None,
                    if_match: None,
                },
            ),
            key(
                &inbox,
                PimdirChangeKind::Remove {
                    handle: PimdirHandle::from("1"),
                    to: Some(archive.clone()),
                    link_id: None,
                    if_match: None,
                },
            ),
            key(
                &inbox,
                PimdirChangeKind::Add {
                    handle: PimdirHandle::from("1"),
                    link_id: None,
                    flags: PimdirFlags::default(),
                    origin: None,
                    object: None,
                },
            ),
            key(
                &inbox,
                PimdirChangeKind::Add {
                    handle: PimdirHandle::from("1"),
                    link_id: Some(PimdirLinkId::from("mid")),
                    flags: PimdirFlags::default(),
                    origin: None,
                    object: None,
                },
            ),
            key(
                &inbox,
                PimdirChangeKind::Add {
                    handle: PimdirHandle::from("1"),
                    link_id: None,
                    flags: PimdirFlags::default(),
                    origin: Some(PimdirOrigin {
                        collection: archive.clone(),
                        handle: PimdirHandle::from("9"),
                    }),
                    object: None,
                },
            ),
            key(
                &inbox,
                PimdirChangeKind::Add {
                    handle: PimdirHandle::from("1"),
                    link_id: None,
                    flags: PimdirFlags::default(),
                    origin: None,
                    object: Some(PimdirHash::from("aaa")),
                },
            ),
        ];

        let distinct: BTreeSet<&PimdirChangeKey> = keys.iter().collect();
        assert_eq!(distinct.len(), keys.len(), "keys collided: {keys:?}");
    }

    #[test]
    fn a_precondition_is_not_part_of_the_key() {
        let inbox = PimdirCollectionId::from("inbox");
        let keyed = key(&inbox, update("1", "aaa", None));

        assert_eq!(key(&inbox, update("1", "aaa", Some("r1"))), keyed);
        assert_eq!(key(&inbox, update("1", "aaa", Some("r2"))), keyed);
    }

    #[test]
    fn an_unknown_flag_set_does_not_key_as_an_empty_one() {
        let inbox = PimdirCollectionId::from("inbox");
        let unknown = PimdirChangeKind::SetFlags {
            handle: PimdirHandle::from("1"),
            flags: PimdirFlags::Unknown,
        };

        assert_ne!(key(&inbox, unknown), key(&inbox, set_flags("1", &[])));
    }
}
