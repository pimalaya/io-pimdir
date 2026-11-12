//! # Versions
//!
//! Whether a program opens a store, judged from the versions the store
//! states and nothing else (STORAGE §4.2): I/O-free, so every profile,
//! and an owner holding its own SQLite, asks one judgement.
//!
//! A draft's store opens under that draft alone, its `format_version`
//! compared to the word. Once the storage part is released, the three
//! numbers of `store_meta` govern: an owner opens a store whose `version`
//! it knows, a reader one whose `read_version` is at most its own schema
//! version, a producer one whose `read_version` and `produce_version` both
//! are; a release refuses every draft store.
//!
//! The judgement reads the stamps alone: the two schema stamps agreeing,
//! the algorithm and the schema's text are the opener's to check beside
//! it (STORAGE §6).

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
    /// A draft, by the name its stores carry (`storage-draft-10`).
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
}

/// The versions a store states (`load_store_meta`, STORAGE §4.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PimdirStoreVersion<'a> {
    /// `store_meta.format_version`, `None` for a store lacking the
    /// column, which a draft before storage-draft-08 wrote.
    pub format_version: Option<&'a str>,
    /// `store_meta.version`, the schema version.
    pub version: i64,
    /// `store_meta.read_version`, the lowest schema version a program
    /// must implement to read the store.
    pub read_version: i64,
    /// `store_meta.produce_version`, the lowest it must implement to
    /// enqueue into it.
    pub produce_version: i64,
}

impl PimdirStoreVersion<'_> {
    /// Judges whether `program` opens this store (STORAGE §4.2), naming
    /// why when it does not.
    ///
    /// A draft program opens the store of its own draft and no other,
    /// whatever its profile and the three numbers. A released one refuses
    /// a draft store and one lacking the marker, then judges by its
    /// profile: an owner opens a `version` from 1 to its own, the store
    /// below its own being its to migrate; a reader opens a
    /// `read_version` at most its own; a producer needs `produce_version`
    /// at most its own as well.
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
            PimdirFormat::Release if !self.is_released() => return Err(format()),
            PimdirFormat::Release => (),
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
                Err(PimdirVersionRefusal::Unknown {
                    found: self.version,
                    own,
                })
            }
            PimdirProfile::Owner => Ok(()),
            PimdirProfile::Reader => read,
            PimdirProfile::Producer if self.produce_version > own => {
                read.and(Err(PimdirVersionRefusal::Produce {
                    found: self.produce_version,
                    own,
                }))
            }
            PimdirProfile::Producer => read,
        }
    }

    /// Whether the store names a released storage part: the prefix, then
    /// a major of digits alone.
    fn is_released(&self) -> bool {
        self.format_version
            .and_then(|format| format.strip_prefix(RELEASED))
            .is_some_and(|major| !major.is_empty() && major.bytes().all(|b| b.is_ascii_digit()))
    }
}

/// Why a program does not open a store (STORAGE §4.2). A refused store is
/// never reconciled, migrated, rebuilt, written nor deleted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirVersionRefusal {
    /// The store's `format_version` is not one the program opens: another
    /// draft's or a release's for a draft program, a draft's for a
    /// released one, or none at all.
    Format {
        /// The store's `format_version`, `None` when it lacks the column.
        found: Option<String>,
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
            format_version: Some(format_version),
            version,
            read_version,
            produce_version,
        }
    }

    fn release(profile: PimdirProfile, version: i64) -> PimdirProgram<'static> {
        PimdirProgram {
            profile,
            version,
            format: PimdirFormat::Release,
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
            for format in ["storage-draft-10", "storage-v", "storage-v1.0.0", "pimdir"] {
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
                format: PimdirFormat::Draft("storage-draft-10"),
            };

            assert_eq!(store("storage-draft-10", (1, 1, 1)).opens(&draft), Ok(()));
            assert_eq!(store("storage-draft-10", (2, 2, 2)).opens(&draft), Ok(()));
            for format in ["storage-draft-09", "storage-draft-11", "storage-v1"] {
                assert_eq!(
                    store(format, (1, 1, 1)).opens(&draft),
                    Err(PimdirVersionRefusal::Format {
                        found: Some(format.into())
                    })
                );
            }
        }
    }
}
