# 📁 I/O Pimdir [![Documentation](https://img.shields.io/docsrs/io-pimdir?style=flat&logo=docs.rs&logoColor=white)](https://docs.rs/io-pimdir/latest/io_pimdir) [![Matrix](https://img.shields.io/badge/chat-%23pimalaya-blue?style=flat&logo=matrix&logoColor=white)](https://matrix.to/#/#pimalaya:matrix.org) [![Mastodon](https://img.shields.io/badge/news-%40pimalaya-blue?style=flat&logo=mastodon&logoColor=white)](https://fosstodon.org/@pimalaya) [![Sponsor](https://img.shields.io/badge/sponsor-pink?style=flat&logo=github-sponsors&logoColor=white)](https://pimalaya.org/sponsor/)

Rust implementation of the Pimdir standard: the store and the sync engine

This library is composed of 3 feature-gated layers:

- Low-level **I/O-free** core: no_std-compatible schema, encodings, per-kind summaries and the five sync verbs as coroutines, usable anywhere
- Mid-level **std client**: the three profiles the standard names, reader, producer and owner, as handles running the statements against SQLite and the blob files, the owner running the verbs against a connector you provide (requires the `client` feature, enabled by default)
- High-level **CLI**: the `pimdir` binary, the operator tool over a store (requires the `cli` feature)

## Table of contents

- [Features](#features)
- [Specification](#specification)
- [Installation](#installation)
- [Usage](#usage)
- [Examples](#examples)
- [AI policy](https://github.com/pimalaya/.github/blob/master/AI_POLICY.md)
- [License](#license)
- [Social](#social)
- [Contributing](https://github.com/pimalaya/.github/blob/master/CONTRIBUTING.md)
- [Sponsoring](#sponsoring)

## Features

- **One store for mail, contacts and calendars**: a portable SQLite database plus a content-addressed blob directory, readable by any conformant pimdir implementation.
- **Offline-first sync**: five verbs, open, upgrade, mutate, sync and rekey, reconciling the store against IMAP, JMAP, CardDAV or CalDAV through a three-way merge, a push confirmed before local state moves.
- **Several sources per item**: one shared item and a base per source, so a change one server folded in reaches the others on their next sync with no cross-merge.
- **Typed summaries**: what a reader lists from without the body, one table per kind with the people an item names, derived the same way by every writer and checked against the format's vectors.
- **Deduplicated bodies**: each body is stored once by content hash, so a message filed in two mailboxes costs one copy and a body already held is linked without a download.
- **Retention**: an item every source dropped is kept, hidden and restorable until an explicit purge.
- **Action queue**: processes that do not own the store append actions the owner applies exactly once.
- **Change feed**: every row carries a stamp, so an index or a window folds what moved since it last looked.
- **Three roles, three handles**: one owner that writes, any number of producers that enqueue, any number of readers that take no lock.
- **Operator CLI**: inspect a store while a sync runs, read the trash, restore or purge, prune the queue, check consistency, collect what nothing references, dump the store.
- **Conformance suite**: the specification's sync and summary vectors run against the real store in the test suite.

> [!TIP]
> io-pimdir is written in [Rust](https://www.rust-lang.org/) and uses [cargo features](https://doc.rust-lang.org/cargo/reference/features.html) to gate each layer. The default feature set is declared in [Cargo.toml](./Cargo.toml) or on [docs.rs](https://docs.rs/crate/io-pimdir/latest/features).

## Specification

io-pimdir is the reference implementation of the [pimdir](https://github.com/pimalaya/pimdir) standard: the owner store STORAGE.md specifies, with its reader and producer profiles, and the engine SYNC.md describes. The canonical schema and statements are vendored under spec/ and generated into the crate at build time, the summaries follow Annex A, and the engine reproduces the sync vectors. The reference index of SEARCH.md is not implemented yet and will come in a later release.

A client that only lists a store, or only queues an action, needs the reader or the producer handle and nothing of the engine; the standard's GUIDE.md §1 says what each profile owes, and this crate's handles meet it.

## Installation

Install the `pimdir` binary from [crates.io](https://crates.io/crates/io-pimdir) with cargo:

```sh
cargo install io-pimdir --locked --features cli
```

To use io-pimdir as a library, add it to your Cargo.toml: the `cli` feature is not part of the defaults, so a library consumer never compiles the binary or its terminal dependencies.

## Usage

The `pimdir` binary is to a store what `sqlite3` is to a database: an operator tool, not an end-user client. Reads open the store read-only, so inspecting a store mid-sync is always safe. A few real-world invocations:

```sh
pimdir -s ~/mail store info
pimdir -s ~/mail collection list
pimdir -s ~/mail item list INBOX --retained
pimdir -s ~/mail item restore 42
pimdir -s ~/mail item purge --older-than 90d
pimdir -s ~/mail queue list --parked
pimdir -s ~/mail check
```

Run `pimdir --help` for the full command tree and flags, and add `--json` to any command for machine-readable output. The CLI contract lives in [cairn/spec/cli.md](./cairn/spec/cli.md). The whole library API is documented on [docs.rs](https://docs.rs/io-pimdir/latest/io_pimdir).

## Examples

The tests demonstrate real usage: [./tests](./tests) runs every verb against a store, and tests/vectors_sync.rs is a complete connector over the specification's vectors.

## License

This project is licensed under either of:

- [MIT license](LICENSE-MIT)
- [Apache License, Version 2.0](LICENSE-APACHE)

## Social

- Chat on [Matrix](https://matrix.to/#/#pimalaya:matrix.org)
- News on [Mastodon](https://fosstodon.org/@pimalaya) or [RSS](https://fosstodon.org/@pimalaya.rss)
- Mail at [pimalaya.org@posteo.net](mailto:pimalaya.org@posteo.net)

## Sponsoring

[![nlnet](https://nlnet.nl/logo/banner-160x60.png)](https://nlnet.nl/)

Special thanks to the [NLnet foundation](https://nlnet.nl/) and the [European Commission](https://www.ngi.eu/) that have been financially supporting the project for years:

- 2022 → 2023: [NGI Assure](https://nlnet.nl/project/Himalaya/)
- 2023 → 2024: [NGI Zero Entrust](https://nlnet.nl/project/Pimalaya/)
- 2024 → 2026: [NGI Zero Core](https://nlnet.nl/project/Pimalaya-PIM/)
- 2026 → 2027: [NGI Zero Commons Fund](https://nlnet.nl/project/Pimalaya-pimdir/)

This program is part of Pimalaya, free software funded entirely by grants and donations. If you find it useful, consider [sponsoring](https://pimalaya.org/sponsor/) its development:

[![GitHub](https://img.shields.io/badge/-GitHub%20Sponsors-fafbfc?logo=GitHub%20Sponsors)](https://github.com/sponsors/soywod)
[![Ko-fi](https://img.shields.io/badge/-Ko--fi-ff5e5a?logo=Ko-fi&logoColor=ffffff)](https://ko-fi.com/pimalaya)
[![Buy Me a Coffee](https://img.shields.io/badge/-Buy%20Me%20a%20Coffee-ffdd00?logo=Buy%20Me%20A%20Coffee&logoColor=000000)](https://www.buymeacoffee.com/pimalaya)
[![Liberapay](https://img.shields.io/badge/-Liberapay-f6c915?logo=Liberapay&logoColor=222222)](https://liberapay.com/pimalaya)
[![thanks.dev](https://img.shields.io/badge/-thanks.dev-000000?logo=data:image/svg+xml;base64,PHN2ZyB3aWR0aD0iMjQuMDk3IiBoZWlnaHQ9IjE3LjU5NyIgY2xhc3M9InctMzYgbWwtMiBsZzpteC0wIHByaW50Om14LTAgcHJpbnQ6aW52ZXJ0IiB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciPjxwYXRoIGQ9Ik05Ljc4MyAxNy41OTdINy4zOThjLTEuMTY4IDAtMi4wOTItLjI5Ny0yLjc3My0uODktLjY4LS41OTMtMS4wMi0xLjQ2Mi0xLjAyLTIuNjA2di0xLjM0NmMwLTEuMDE4LS4yMjctMS43NS0uNjc4LTIuMTk1LS40NTItLjQ0Ni0xLjIzMi0uNjY5LTIuMzQtLjY2OUgwVjcuNzA1aC41ODdjMS4xMDggMCAxLjg4OC0uMjIyIDIuMzQtLjY2OC40NTEtLjQ0Ni42NzctMS4xNzcuNjc3LTIuMTk1VjMuNDk2YzAtMS4xNDQuMzQtMi4wMTMgMS4wMjEtMi42MDZDNS4zMDUuMjk3IDYuMjMgMCA3LjM5OCAwaDIuMzg1djEuOTg3aC0uOTg1Yy0uMzYxIDAtLjY4OC4wMjctLjk4LjA4MmExLjcxOSAxLjcxOSAwIDAgMC0uNzM2LjMwN2MtLjIwNS4xNTYtLjM1OC4zODQtLjQ2LjY4Mi0uMTAzLjI5OC0uMTU0LjY4Mi0uMTU0IDEuMTUxVjUuMjNjMCAuODY3LS4yNDkgMS41ODYtLjc0NSAyLjE1NS0uNDk3LjU2OS0xLjE1OCAxLjAwNC0xLjk4MyAxLjMwNXYuMjE3Yy44MjUuMyAxLjQ4Ni43MzYgMS45ODMgMS4zMDUuNDk2LjU3Ljc0NSAxLjI4Ny43NDUgMi4xNTR2MS4wMjFjMCAuNDcuMDUxLjg1NC4xNTMgMS4xNTIuMTAzLjI5OC4yNTYuNTI1LjQ2MS42ODIuMTkzLjE1Ny40MzcuMjYuNzMyLjMxMi4yOTUuMDUuNjIzLjA3Ni45ODQuMDc2aC45ODVabTE0LjMxNC03LjcwNmgtLjU4OGMtMS4xMDggMC0xLjg4OC4yMjMtMi4zNC42NjktLjQ1LjQ0NS0uNjc3IDEuMTc3LS42NzcgMi4xOTVWMTQuMWMwIDEuMTQ0LS4zNCAyLjAxMy0xLjAyIDIuNjA2LS42OC41OTMtMS42MDUuODktMi43NzQuODloLTIuMzg0di0xLjk4OGguOTg0Yy4zNjIgMCAuNjg4LS4wMjcuOTgtLjA4LjI5Mi0uMDU1LjUzOC0uMTU3LjczNy0uMzA4LjIwNC0uMTU3LjM1OC0uMzg0LjQ2LS42ODIuMTAzLS4yOTguMTU0LS42ODIuMTU0LTEuMTUydi0xLjAyYzAtLjg2OC4yNDgtMS41ODYuNzQ1LTIuMTU1LjQ5Ny0uNTcgMS4xNTgtMS4wMDQgMS45ODMtMS4zMDV2LS4yMTdjLS44MjUtLjMwMS0xLjQ4Ni0uNzM2LTEuOTgzLTEuMzA1LS40OTctLjU3LS43NDUtMS4yODgtLjc0NS0yLjE1NXYtMS4wMmMwLS40Ny0uMDUxLS44NTQtLjE1NC0xLjE1Mi0uMTAyLS4yOTgtLjI1Ni0uNTI2LS40Ni0uNjgyYTEuNzE5IDEuNzE5IDAgMCAwLS43MzctLjMwNyA1LjM5NSA1LjM5NSAwIDAgMC0uOTgtLjA4MmgtLjk4NFYwaDIuMzg0YzEuMTY5IDAgMi4wOTMuMjk3IDIuNzc0Ljg5LjY4LjU5MyAxLjAyIDEuNDYyIDEuMDIgMi42MDZ2MS4zNDZjMCAxLjAxOC4yMjYgMS43NS42NzggMi4xOTUuNDUxLjQ0NiAxLjIzMS42NjggMi4zNC42NjhoLjU4N3oiIGZpbGw9IiNmZmYiLz48L3N2Zz4=)](https://thanks.dev/u/gh/soywod)
[![PayPal](https://img.shields.io/badge/-PayPal-0079c1?logo=PayPal&logoColor=ffffff)](https://www.paypal.com/paypalme/soywod)
