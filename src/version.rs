//! # Versions
//!
//! Whether a program opens a store, judged from the versions the store
//! states and nothing else (STORAGE §4.2): I/O-free, so every profile,
//! and an owner holding its own SQLite, asks one judgement.
//!
//! A draft's store opens under that draft alone, its `format_version`
//! compared to the word; the two schema stamps agreeing, the algorithm
//! and the schema's text are then the opener's to check beside it
//! (STORAGE §6).
//!
//! Once the storage part is released, a store is judged by six columns of
//! `store_meta` and by nothing else of the schema, one above the
//! program's version included (STORAGE §4.2): `format` is `pimdir`;
//! `format_version` is `storage-v` followed by `version`, a draft's name
//! refused; `version` equals `PRAGMA user_version`; `read_version`, and
//! for a producer `produce_version` too, is at most the program's own; an
//! owner opens a `version` it knows; `hash_algo` is one the program
//! knows. A table, a column, an index, a trigger or a view the program
//! does not know is a later version's, never damage: the schema's text is
//! compared against a version the program knows alone (§18).

use alloc::string::{String, ToString};
use core::fmt;

/// The prefix of the `format_version` a released storage part writes,
/// its major following (STORAGE §4.2).
const RELEASED: &str = "storage-v";

/// The profile a program opens a store under (STORAGE §17), each holding
/// the one before.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirProfile {
    /// Opens the store read-only and lists it.
    Reader,
    /// A reader that enqueues actions.
    Producer,
    /// The one process that mutates the store.
    Owner,
}

/// The storage part a program implements (STORAGE §4.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirFormat<'a> {
    /// A draft, by the name its stores carry (`storage-draft-11`).
    Draft(&'a str),
    /// A released version.
    Release,
}

/// A program as the judgement reads it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PimdirProgram<'a> {
    /// The profile it opens the store under.
    pub profile: PimdirProfile,
    /// The schema version it implements, its storage part's major.
    pub version: i64,
    /// Whether it is built from a draft or a release.
    pub format: PimdirFormat<'a>,
    /// The hash algorithms it knows, by their `store_meta.hash_algo`
    /// name (STORAGE §5).
    pub hash_algos: &'a [&'a str],
}

/// What a store states of itself: the six columns of `store_meta` a
/// program judges it by (`load_store_meta`, STORAGE §4.2), which keep
/// their name, type and meaning in every version, beside the
/// `PRAGMA user_version` the schema version is compared with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PimdirStoreVersion<'a> {
    /// `store_meta.format`, `None` for a database lacking the column.
    pub format: Option<&'a str>,
    /// `store_meta.format_version`, `None` for a store lacking the
    /// column, which a draft before storage-draft-08 wrote.
    pub format_version: Option<&'a str>,
    /// `store_meta.version`, the schema version.
    pub version: i64,
    /// `PRAGMA user_version`, which a sound store states equal to
    /// `version`.
    pub user_version: i64,
    /// `store_meta.read_version`, the lowest schema version a program
    /// must implement to read the store.
    pub read_version: i64,
    /// `store_meta.produce_version`, the lowest it must implement to
    /// enqueue into it.
    pub produce_version: i64,
    /// `store_meta.hash_algo`, the algorithm naming its bodies.
    pub hash_algo: &'a str,
}

impl PimdirStoreVersion<'_> {
    /// Judges whether `program` opens this store (STORAGE §4.2), naming
    /// why when it does not.
    ///
    /// A draft program opens the store of its own draft and no other,
    /// whatever its profile and the other columns, which its opener
    /// checks beside. A released one judges by the six columns, in this
    /// order: a `format` other than `pimdir` is no store; a draft store,
    /// one lacking the marker and one whose marker is not `storage-v`
    /// followed by its `version` are refused; then a `version` other
    /// than `user_version`; then its profile, an owner opening a
    /// `version` from 1 to its own, the store below its own being its to
    /// migrate, a reader a `read_version` at most its own, a producer
    /// needing `produce_version` at most its own as well; then an
    /// algorithm it does not know. A `version` above a reader's or a
    /// producer's own refuses nothing by itself.
    pub fn opens(&self, program: &PimdirProgram<'_>) -> Result<(), PimdirVersionRefusal> {
        let format = || PimdirVersionRefusal::Format {
            found: self.format_version.map(ToString::to_string),
        };

        match program.format {
            PimdirFormat::Draft(draft) => {
                return match self.format_version == Some(draft) {
                    true => Ok(()),
                    false => Err(format()),
                };
            }
            PimdirFormat::Release if self.format != Some("pimdir") => {
                return Err(PimdirVersionRefusal::Foreign {
                    found: self.format.map(ToString::to_string),
                });
            }
            PimdirFormat::Release => (),
        }
        // NOTE: the marker names the schema version, `storage-v1` for
        // version 1, so a draft's name and a marker left behind by a
        // migration are refused alike.
        if self.released() != Some(self.version) {
            return Err(format());
        }
        if self.version != self.user_version {
            return Err(PimdirVersionRefusal::Mismatch {
                user_version: self.user_version,
                version: self.version,
            });
        }

        let own = program.version;
        let read = match self.read_version > own {
            true => Err(PimdirVersionRefusal::Read {
                found: self.read_version,
                own,
            }),
            false => Ok(()),
        };

        match program.profile {
            PimdirProfile::Owner if self.version < 1 || self.version > own => {
                return Err(PimdirVersionRefusal::Unknown {
                    found: self.version,
                    own,
                });
            }
            PimdirProfile::Owner => (),
            PimdirProfile::Reader => read?,
            PimdirProfile::Producer if self.produce_version > own => {
                read?;
                return Err(PimdirVersionRefusal::Produce {
                    found: self.produce_version,
                    own,
                });
            }
            PimdirProfile::Producer => read?,
        }

        match program.hash_algos.contains(&self.hash_algo) {
            true => Ok(()),
            false => Err(PimdirVersionRefusal::HashAlgo {
                found: self.hash_algo.to_string(),
            }),
        }
    }

    /// The major the store's marker names when it is a released storage
    /// part's: the prefix, then digits alone, no sign and no leading
    /// zero, so one schema version has one marker.
    fn released(&self) -> Option<i64> {
        let major = self.format_version?.strip_prefix(RELEASED)?;
        let canonical = major.bytes().all(|b| b.is_ascii_digit()) && !major.starts_with('0');
        canonical.then(|| major.parse().ok()).flatten()
    }
}

/// Why a program does not open a store (STORAGE §4.2). A refused store is
/// never reconciled, migrated, rebuilt, written nor deleted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirVersionRefusal {
    /// The store's `format` is not `pimdir`: another program's database,
    /// to a released program.
    Foreign {
        /// The store's `format`, `None` when it lacks the column.
        found: Option<String>,
    },
    /// The store's `format_version` is not one the program opens: another
    /// draft's or a release's for a draft program; for a released one a
    /// draft's, one that is not `storage-v` followed by the store's
    /// `version`, or none at all.
    Format {
        /// The store's `format_version`, `None` when it lacks the column.
        found: Option<String>,
    },
    /// The store's `version` is not its `PRAGMA user_version`, to a
    /// released program.
    Mismatch {
        /// The database's `PRAGMA user_version`.
        user_version: i64,
        /// The store's `version`.
        version: i64,
    },
    /// The store names its bodies by an algorithm a released program does
    /// not know.
    HashAlgo {
        /// The store's `hash_algo`.
        found: String,
    },
    /// An owner does not know the store's schema version: above its own.
    Unknown {
        /// The store's `version`.
        found: i64,
        /// The program's schema version.
        own: i64,
    },
    /// Reading the store takes a later schema version than the program's.
    Read {
        /// The store's `read_version`.
        found: i64,
        /// The program's schema version.
        own: i64,
    },
    /// Enqueueing into the store takes a later schema version than the
    /// program's, which may still read it.
    Produce {
        /// The store's `produce_version`.
        found: i64,
        /// The program's schema version.
        own: i64,
    },
}

impl fmt::Display for PimdirVersionRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Format { found: Some(found) } => {
                write!(
                    f,
                    "Store format version {found} is not one this program opens"
                )
            }
            Self::Format { found: None } => write!(f, "Store states no format version"),
            Self::Foreign { found: Some(found) } => {
                write!(f, "Database of format {found} is no pimdir store")
            }
            Self::Foreign { found: None } => write!(f, "Database states no format"),
            Self::Mismatch {
                user_version,
                version,
            } => write!(
                f,
                "Store schema version {version} is not its user_version {user_version}"
            ),
            Self::HashAlgo { found } => {
                write!(f, "Store hash algorithm {found} is unknown to this program")
            }
            Self::Unknown { found, own } => write!(
                f,
                "Store schema version {found} is unknown to an owner at version {own}"
            ),
            Self::Read { found, own } => write!(
                f,
                "Store takes schema version {found} to read, this program is at {own}"
            ),
            Self::Produce { found, own } => write!(
                f,
                "Store takes schema version {found} to enqueue, this program is at {own}"
            ),
        }
    }
}

impl core::error::Error for PimdirVersionRefusal {}

#[cfg(test)]
mod tests {
    use crate::version::*;

    const PROFILES: [PimdirProfile; 3] = [
        PimdirProfile::Reader,
        PimdirProfile::Producer,
        PimdirProfile::Owner,
    ];

    fn store(
        format_version: &str,
        (version, read_version, produce_version): (i64, i64, i64),
    ) -> PimdirStoreVersion<'_> {
        PimdirStoreVersion {
            format: Some("pimdir"),
            format_version: Some(format_version),
            version,
            user_version: version,
            read_version,
            produce_version,
            hash_algo: "blake3",
        }
    }

    fn release(profile: PimdirProfile, version: i64) -> PimdirProgram<'static> {
        PimdirProgram {
            profile,
            version,
            format: PimdirFormat::Release,
            hash_algos: &["blake3"],
        }
    }

    /// A release judges a store by the six columns (STORAGE §4.2): each
    /// one off refuses it under every profile, a version 1 program on a
    /// version 2 store that raised no floor included, and nothing else
    /// of that store does.
    #[test]
    fn a_release_judges_a_store_by_six_columns() {
        use PimdirVersionRefusal::*;

        let sound = store("storage-v2", (2, 1, 1));
        for profile in [PimdirProfile::Reader, PimdirProfile::Producer] {
            let program = release(profile, 1);
            assert_eq!(sound.opens(&program), Ok(()));

            let foreign = PimdirStoreVersion {
                format: Some("other"),
                ..sound
            };
            assert_eq!(
                foreign.opens(&program),
                Err(Foreign {
                    found: Some("other".into())
                })
            );

            // NOTE: the marker is `storage-v` followed by `version`: one a
            // migration left behind, or ahead, is refused.
            for marker in ["storage-v1", "storage-v3", "storage-v02", "storage-v+2"] {
                let stale = PimdirStoreVersion {
                    format_version: Some(marker),
                    ..sound
                };
                assert_eq!(
                    stale.opens(&program),
                    Err(Format {
                        found: Some(marker.into())
                    })
                );
            }

            let disagreeing = PimdirStoreVersion {
                user_version: 1,
                ..sound
            };
            assert_eq!(
                disagreeing.opens(&program),
                Err(Mismatch {
                    user_version: 1,
                    version: 2
                })
            );

            let hashed = PimdirStoreVersion {
                hash_algo: "sha256",
                ..sound
            };
            assert_eq!(
                hashed.opens(&program),
                Err(HashAlgo {
                    found: "sha256".into()
                })
            );
        }
    }

    /// A version 1 release is the older program of every later store: it
    /// reads and enqueues into a version 2 store that raised neither
    /// floor, reads one that raised the produce floor alone, opens none
    /// that raised the read floor, and never owns one.
    #[test]
    fn a_version_1_release_judges_a_version_2_store_by_its_floors() {
        use PimdirVersionRefusal::*;

        let own = 1;
        let cases = [
            ((2, 1, 1), [Ok(()), Ok(()), Err(Unknown { found: 2, own })]),
            (
                (2, 2, 1),
                [
                    Err(Read { found: 2, own }),
                    Err(Read { found: 2, own }),
                    Err(Unknown { found: 2, own }),
                ],
            ),
            (
                (2, 1, 2),
                [
                    Ok(()),
                    Err(Produce { found: 2, own }),
                    Err(Unknown { found: 2, own }),
                ],
            ),
        ];

        for (versions, expected) in cases {
            let store = store("storage-v2", versions);
            for (profile, expected) in PROFILES.into_iter().zip(expected) {
                assert_eq!(
                    store.opens(&release(profile, own)),
                    expected,
                    "{profile:?} at {own} on {versions:?}"
                );
            }
        }
    }

    /// A release opens every store up to its own version under every
    /// profile, the owner to migrate the older one.
    #[test]
    fn a_release_opens_the_stores_it_knows() {
        for profile in PROFILES {
            for (format, versions) in [
                ("storage-v1", (1, 1, 1)),
                ("storage-v2", (2, 1, 1)),
                ("storage-v2", (2, 2, 2)),
            ] {
                assert_eq!(store(format, versions).opens(&release(profile, 2)), Ok(()));
            }
        }

        let unversioned = store("storage-v1", (0, 1, 1));
        assert!(
            unversioned
                .opens(&release(PimdirProfile::Owner, 2))
                .is_err()
        );
    }

    /// A release refuses every draft store, one lacking the marker and one
    /// whose marker names no release, whatever the numbers say.
    #[test]
    fn a_release_refuses_a_draft_store() {
        for profile in PROFILES {
            for format in ["storage-draft-11", "storage-v", "storage-v1.0.0", "pimdir"] {
                assert_eq!(
                    store(format, (1, 1, 1)).opens(&release(profile, 1)),
                    Err(PimdirVersionRefusal::Format {
                        found: Some(format.into())
                    })
                );
            }

            let unmarked = PimdirStoreVersion {
                format_version: None,
                ..store("", (1, 1, 1))
            };
            assert_eq!(
                unmarked.opens(&release(profile, 1)),
                Err(PimdirVersionRefusal::Format { found: None })
            );
        }
    }

    /// A draft opens its exact draft and nothing else, under every
    /// profile and whatever the numbers say.
    #[test]
    fn a_draft_opens_its_own_draft_alone() {
        for profile in PROFILES {
            let draft = PimdirProgram {
                profile,
                version: 1,
                format: PimdirFormat::Draft("storage-draft-11"),
                hash_algos: &["blake3"],
            };

            assert_eq!(store("storage-draft-11", (1, 1, 1)).opens(&draft), Ok(()));
            assert_eq!(store("storage-draft-11", (2, 2, 2)).opens(&draft), Ok(()));
            for format in ["storage-draft-10", "storage-draft-12", "storage-v1"] {
                assert_eq!(
                    store(format, (1, 1, 1)).opens(&draft),
                    Err(PimdirVersionRefusal::Format {
                        found: Some(format.into())
                    })
                );
            }

            // NOTE: a draft reads the marker alone, the other columns
            // being its opener's to check (STORAGE §6).
            let other = PimdirStoreVersion {
                format: None,
                user_version: 7,
                hash_algo: "sha256",
                ..store("storage-draft-11", (1, 1, 1))
            };
            assert_eq!(other.opens(&draft), Ok(()));
        }
    }
}
