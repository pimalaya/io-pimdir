# I/O Pimdir [![Documentation](https://img.shields.io/docsrs/io-pimdir?style=flat&logo=docs.rs&logoColor=white)](https://docs.rs/io-pimdir/latest/io_pimdir) [![Matrix](https://img.shields.io/badge/chat-%23pimalaya-blue?style=flat&logo=matrix&logoColor=white)](https://matrix.to/#/#pimalaya:matrix.org) [![Mastodon](https://img.shields.io/badge/news-%40pimalaya-blue?style=flat&logo=mastodon&logoColor=white)](https://fosstodon.org/@pimalaya) [![Sponsor](https://img.shields.io/badge/sponsor-pink?style=flat&logo=github-sponsors&logoColor=white)](https://pimalaya.org/sponsor/)

Rust implementation of [Pimdir](https://github.com/pimalaya/pimdir) standard

This project is composed of 3 feature-gated layers:

- Low-level **I/O-free** core: no_std-compatible schema, statements and model-to-column encodings, reusable by any implementation
- Mid-level **std client**: `PimdirStore`, `PimdirProducer` and `PimdirReader`, the owner, producer and reader handles running the statements against SQLite and the blob files, servicing the io-replica storage seam
- High-level **CLI**: the `pimdir` binary, the operator front-end over a store (requires the `cli` feature)

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

- **Portable store**: a single SQLite index plus a content-addressed blob directory, readable by any conformant pimdir implementation.
- **Deduplicated bodies**: each body is stored once by content hash, so a message filed in two mailboxes costs one copy.
- **Offline-first**: keeps the shared item and a per-source base, the raw material a sync engine reconciles against.
- **Short public ids**: one small, store-global id per message, shared across every collection and never reused.
- **Crash-safe writes**: one transaction per batch, bodies durable before the rows that reference them, and blobs garbage collected inside it.
- **Retention**: a removal retires an item instead of destroying it, hidden from every read and from the sync, until an explicit purge reclaims it.
- **Action queue**: processes that do not own the store request mutations by appending actions the owner applies exactly once, with parked failures queryable and collection generations carrying the handle-space epoch to readers.
- **Three roles, three handles**: one owner that writes, any number of producers that enqueue, and any number of readers that take no lock and carry no write at all, so a frontend cannot drain or sweep a store it only reads.
- **Read-your-writes**: a reader folds the queue over the committed items on request, so a staged flag, removal, move or copy shows before the owner applies it, while a queued creation is reported apart, having no public id yet.
- **Operator CLI**: inspect a store while a sync is running, read the trash, restore or purge an item, prune the queue, check consistency, collect what nothing references and dump the whole store (requires the `cli` feature).
- **no_std core**: the schema, statements and encodings need no allocator beyond `alloc` and pull SQLite in only behind the `client` feature.

> [!TIP]
> io-pimdir is written in [Rust](https://www.rust-lang.org/) and uses [cargo features](https://doc.rust-lang.org/cargo/reference/features.html) to gate each layer. The default feature set is declared in [Cargo.toml](./Cargo.toml) or on [docs.rs](https://docs.rs/crate/io-pimdir/latest/features).

## Specification

io-pimdir implements the [pimdir](https://github.com/pimalaya/pimdir) on-disk store specification: a SQLite database plus a content-addressed blob directory, with a canonical schema and forward-only migrations. The spec is the cross-implementation contract, so a store written here is readable by any other conformant implementation (a native Android SQLite store, for example). The sync model it services (a shared item, a per-source base, detail levels, conflicts) lives in [io-replica](https://github.com/pimalaya/io-replica).

## Installation

The CLI binary pimdir has not been officially released yet. Install it from [crates.io](https://crates.io/crates/io-pimdir) with cargo:

```sh
cargo install io-pimdir --locked --features cli
```

To use io-pimdir as a library, add it to your Cargo.toml: the `cli` feature is not part of the defaults, so a library consumer never compiles the binary or its terminal dependencies.

## Usage

The `pimdir` binary is to a store what `sqlite3` is to a database: an operator and debugging tool, not an end-user client. It is kind-agnostic and never interprets item content, so it prints ids, flags, levels and the raw meta, and exports raw bytes; rendering a message or a contact belongs to [himalaya](https://github.com/pimalaya/himalaya) and [cardamum](https://github.com/pimalaya/cardamum). Reads open the store read-only, so inspecting a store mid-sync is always safe. A few real-world invocations:

```sh
pimdir -s ~/mail store info
pimdir -s ~/mail collection list
pimdir -s ~/mail item list INBOX --retained
pimdir -s ~/mail item restore 42
pimdir -s ~/mail item purge --older-than 90d
pimdir -s ~/mail queue list --parked
pimdir -s ~/mail check
```

Run `pimdir --help` for the full command tree and flags, and add `--json` to any command for machine-readable output. The CLI contract (the roles it opens a store with, its verb surface, its confirmation rules) lives in [cairn/spec/cli.md](./cairn/spec/cli.md). See documentation at [docs.rs](https://docs.rs/io-pimdir/latest/io_pimdir).

## Examples

See complete examples at [./tests](./tests).

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
