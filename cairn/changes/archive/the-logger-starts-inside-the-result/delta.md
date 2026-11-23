---
cairn: delta
change: the-logger-starts-inside-the-result
---

## ADDED Requirements

### Requirement: A logger failing to start is an error
The `pimdir` binary SHALL start its logger inside the command's result, so a `--log-file` it cannot open is reported on the output contract, exit 1, never a panic.

## MODIFIED Requirements

## REMOVED Requirements
