---
cairn: change
id: the-logger-starts-inside-the-result
status: landed
created: 2026-11-19
---

# The logger starts inside the result

## Why

PIM-C2, found on 2026-11-19 across the family: the `pimdir` binary started its logger with `Logger::try_init(&cli.log).expect("init logger")` (src/main.rs), so a `--log-file` it cannot open panicked, exit 101 and nothing on stdout, outside the output contract (exit 1 and a JSON error). ortie fixed the same by starting the logger inside the result.

## What

`Logger::try_init(&cli.log).and_then(|()| cli.command.execute(..))`, the result handed to `ErrorReport::eval_coded` as before.
