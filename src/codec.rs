//! Pure, I/O-free encodings between the [`io_replica`] model and the pimdir
//! columns (spec §11). No SQLite, no filesystem: this is the part an Android or
//! any other implementation reuses.

use alloc::{string::String, vec::Vec};

use io_replica::placement::{ReplicaFlags, ReplicaLevel};

/// A flag set to its canonical JSON array (sorted; the model's set is already
/// ordered). An empty set encodes as `"[]"`, never `NULL`.
pub fn flags_to_json(flags: &ReplicaFlags) -> String {
    let items: Vec<&String> = flags.0.iter().collect();
    serde_json::to_string(&items).unwrap_or_else(|_| String::from("[]"))
}

/// The inverse of [`flags_to_json`]. A `NULL`/absent column (passed as `None`)
/// or malformed JSON decodes to the empty set — the model has no "unknown flags"
/// state.
pub fn flags_from_json(json: Option<&str>) -> ReplicaFlags {
    let Some(json) = json else {
        return ReplicaFlags::default();
    };
    let items: Vec<String> = serde_json::from_str(json).unwrap_or_default();
    ReplicaFlags(items.into_iter().collect())
}

/// The detail ladder as its column integer (spec §11).
pub fn level_to_int(level: ReplicaLevel) -> i64 {
    match level {
        ReplicaLevel::Probed => 0,
        ReplicaLevel::Meta => 1,
        ReplicaLevel::Full => 2,
    }
}

/// The inverse of [`level_to_int`]; unknown integers clamp to `Probed`.
pub fn level_from_int(value: i64) -> ReplicaLevel {
    match value {
        1 => ReplicaLevel::Meta,
        2 => ReplicaLevel::Full,
        _ => ReplicaLevel::Probed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_round_trip_and_escape() {
        let flags = ReplicaFlags::from_iter(["\\Seen", "$flagged", "a\"b"]);
        let json = flags_to_json(&flags);
        assert_eq!(flags_from_json(Some(&json)), flags);
        // NOTE: canonical (sorted) and JSON-escaped.
        assert!(json.starts_with('['));
        assert!(json.contains("\\\\Seen"));
    }

    #[test]
    fn empty_and_null_flags_are_empty() {
        assert_eq!(flags_to_json(&ReplicaFlags::default()), "[]");
        assert_eq!(flags_from_json(None), ReplicaFlags::default());
        assert_eq!(flags_from_json(Some("[]")), ReplicaFlags::default());
    }

    #[test]
    fn level_map_round_trips() {
        for l in [ReplicaLevel::Probed, ReplicaLevel::Meta, ReplicaLevel::Full] {
            assert_eq!(level_from_int(level_to_int(l)), l);
        }
    }
}
