---
cairn: log
change: the-logger-starts-inside-the-result
landed: 2026-11-19
---

# The logger starts inside the result

PIM-C2: the `pimdir` binary panicked (exit 101, nothing printed) on a `--log-file` it could not open, its logger started with `expect`. src/main.rs now starts it inside the result handed to `ErrorReport::eval_coded`, as ortie does: the failure is an error on the output contract, exit 1.

Capabilities moved: cli ("A logger failing to start is an error").

Test: tests/cli_json.rs `an_unopenable_log_file_is_an_error_not_a_panic`, failing on e78ef92's src/main.rs (exit 101) and passing after.
