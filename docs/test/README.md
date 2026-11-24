# Test262 results

Generated reports for the [Test262](https://github.com/tc39/test262) conformance
suite. **Everything in this directory is generated — do not edit it by hand.**

| File                                         | Contents                                                                    |
| -------------------------------------------- | --------------------------------------------------------------------------- |
| [`master.md`](master.md)                     | Aggregated results for every category, plus the full appendix of every test |
| [`language/master.md`](language/master.md)   | `test262/test/language/`                                                    |
| [`built-ins/master.md`](built-ins/master.md) | `test262/test/built-ins/`                                                   |
| [`harness/master.md`](harness/master.md)     | `test262/test/harness/`                                                     |
| [`intl402/master.md`](intl402/master.md)     | `test262/test/intl402/`                                                     |
| [`staging/master.md`](staging/master.md)     | `test262/test/staging/`                                                     |
| [`annexB/master.md`](annexB/master.md)       | `test262/test/annexB/`                                                      |

Each report contains:

1. a summary (counts, pass rate, wall-clock duration),
2. a per-subdirectory breakdown,
3. failures collapsed by error signature,
4. an appendix table listing **every** discovered test with its relative path,
   status and the error message or skip reason produced by the runner.

## Regenerating

```bash
# one category
python3 scripts/test/test262-language.py

# everything (writes docs/test/master.md as well)
python3 scripts/test/test262-all.py --jobs 12
```

Useful flags (all category scripts and the wrapper accept them):

| Flag                | Meaning                                                         |
| ------------------- | --------------------------------------------------------------- |
| `--jobs N`          | chunk subprocesses to run in parallel (default 8)               |
| `--only SUBSTR`     | only run chunks whose path contains `SUBSTR` (repeatable)       |
| `--timeout-ms N`    | per-test watchdog timeout                                       |
| `--chunk-timeout N` | wall-clock timeout for one chunk                                |
| `--strict`          | exit non-zero when at least one test failed                     |
| `--build`           | build the runner if the binary is missing                       |
| `--docs DIR`        | output directory (default `docs/test`)                          |
| `--logs DIR`        | raw runner log directory (default `~/.cache/v16/test262-logs`) |
| `--log-all`         | also keep logs for chunks that passed                           |

The scripts drive `examples/test262_runner`, which must be built first:

```bash
cargo build -r --example test262_runner
```

Raw per-chunk logs are written **outside the repository** (`~/.cache/v16/test262-logs/`
by default, or `$V16_TEST262_LOGS`). Only chunks with failures or warnings are
saved unless `--log-all` is passed. Keeping logs out of the workspace matters:
the run produces thousands of files, and a file watcher indexing them will
destabilise VS Code. `.vscode/settings.json` additionally excludes `test262/`,
`target/`, `bench-v8/` and `docs/test/` from the watcher and the search index.
