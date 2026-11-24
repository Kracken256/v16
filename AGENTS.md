# AGENTS.md

Guidance for agentic coding tools working in this repository. Read this before
touching code. The project-specific rules are at the bottom under
[Non-negotiables](#non-negotiables) — do not skip them.

## What this project is

**v16** is a fast, minimal **ES2023 JavaScript engine written from
scratch in Rust**. It is a full pipeline — hand-written lexer/parser → AST →
register-based bytecode → VM — with a nursery/generational GC, hidden-class
shapes, inline caches, and zero-dependency built-ins (regular expressions, JSON,
Base64, BigInt, Unicode tables, `fetch`, WebSocket, Server-Sent Events,
`process`).

| Entity        | Value                                                            |
| ------------- | ---------------------------------------------------------------- |
| Crate         | `v16-js` (published on crates.io)                               |
| Library name  | `v16` (`use v16::...`)                                         |
| Binary name   | `v16` (`bin/v16.rs`)                                           |
| Edition       | 2024 (nightly toolchain)                                         |
| Default feats | `repl` → implies `fetch`, `process`                              |
| License       | MIT                                                              |

## Repository layout

| Path              | Purpose                                                                          |
| ----------------- | -------------------------------------------------------------------------------- |
| `src/compiler/`   | Lexer, parser, early-error validation, codegen, peephole optimizer, register opcodes, inline caches, MIR. |
| `src/runtime/`    | `JSRuntime` (shared, owns atoms + GC + modules), `JSContext` (per-execution state), VM, event loop, GC, modules, host extensions. |
| `src/object/`     | `JSObject`, `JSFunction`, arrays, hidden-class `Shape`/`ShapeCache`, native function wrappers. |
| `src/builtins/`   | Every standard-library object (`array.rs`, `string.rs`, `regexp.rs`, `promise.rs`, …) plus `global.rs` (the global environment). |
| `src/regexp/`     | Self-contained regex engine: parser, compiler, DFA/backtracking engines, optimizers. |
| `src/http/`       | Zero-dependency HTTP client, TLS glue, chunked/compression, WebSocket client.     |
| `src/host/`       | Embedding API types: `HostFunction`, `HostContext`, `ToJS`/`FromJS`.             |
| `src/util/`       | Small helpers (FxHash, memchr, io multiplexing).                                 |
| `src/value.rs`    | `JSValue`: a 64-bit NaN-boxed tagged value.                                      |
| `bin/v16.rs`     | CLI: run scripts, compile/disassemble `.jsc`, REPL.                              |
| `tests/`          | Rust integration tests (`cargo test`), 769 `#[test]` functions across ~45 files. |
| `examples/`       | Rust examples, the Test262 runner, and hand-run `.js` scripts.                    |
| `scripts/test/`   | Python drivers that run Test262 by category and generate Markdown reports.       |
| `docs/test/`      | **Generated** Test262 reports. Never edit by hand. See `docs/test/README.md`.    |
| `test262/`        | Vendored Test262 suite (git-ignored). Not present in a fresh clone.              |
| `bench-v8.sh`     | V8 benchmark harness comparing `v16` against `qjs`, `node`, `boa`.              |
| `test262.sh`      | Clones Test262 (if missing) and runs the example runner over the whole suite.    |
| `test262_plan.md` | Historical engineering log with the project's working principles.                |

## Build

```bash
# Default: REPL + fetch + process support.
cargo build --release

# Minimal engine (no REPL, no fetch, no process).
cargo build --release --no-default-features

# Pick features explicitly.
cargo build --release --no-default-features --features fetch
cargo build --release --no-default-features --features process
```

Features (`Cargo.toml`):

- `repl` (default) → `fetch` + `process` + `rustyline`. Turns on the interactive
  REPL when `v16` is run with no arguments.
- `fetch` → `rustls`, `webpki-roots`, `flate2`, `libc`. Enables `src/http/`,
  `fetch`, `WebSocket`, `EventSource`.
- `process` → `process.exec/cwd/exit` built-ins.
- `full_runtime_tests` → compiles `tests/test262_harness.rs` (an in-process
  Test262 harness). Off by default.

`Cargo.toml` sets `panic = "abort"` and `lto = true` in release; the VM relies on
this and on a large thread stack (see `examples/test262_runner.rs`).

## Running

```bash
# Run a script.
cargo run --release -- script.js
# or the built binary:
./target/release/v16 script.js

# Optimization level (O0..O3, default O2).
./target/release/v16 -O3 script.js

# Bytecode: compile, disassemble, then execute.
./target/release/v16 -compile input.js output.jsc
./target/release/v16 -diss output.jsc
./target/release/v16 output.jsc

# REPL (needs the `repl` feature, on by default).
./target/release/v16
```

Available JS globals include `console.{log,error,warn,info}`, `setTimeout`,
`setInterval`, `clearTimeout`, `clearInterval`, `queueMicrotask`, `print`,
`alert`, `import(specifier)` (dynamic ESM import), `fetch`, `WebSocket`,
`EventSource`, `btoa`/`atob`, `escape`/`unescape`, and (with `process`)
`process.{exec,cwd,exit,argv,env}`.

## Testing

### Rust test suite

```bash
cargo test                     # all unit + integration tests
cargo test --release           # perf-sensitive tests
cargo test --features full_runtime_tests          # include in-process test262 harness
cargo test --features full_runtime_tests test262_harness
cargo test --test array        # one integration file
cargo test test_math_abs       # one test by name
```

Unit tests live beside the code (`#[cfg(test)] mod tests`, e.g.
`src/regexp/tests.rs`); integration tests live in `tests/`. `tests/readme_examples.rs`
mirrors the README's Rust snippets — **if you change an example in `README.md`,
update that file too** or the build fails.

### Test262 conformance

The example runner `examples/test262_runner.rs` drives the vendored Test262
suite. It prints only the first 50 passing tests; the Python scripts in
`scripts/test/` reconstruct per-test status by parsing the runner's
`V16_T262_TRACE=1` output.

```bash
# 1. Get the suite (test262/ is git-ignored).
./test262.sh                       # clones Test262 and runs the whole suite

# 2. Or drive it manually:
cargo build -r --example test262_runner
./target/release/examples/test262_runner test262/test test262/harness

# 3. Generate Markdown reports (writes docs/test/).
python3 scripts/test/test262-language.py           # one category
python3 scripts/test/test262-built-ins.py
python3 scripts/test/test262-all.py --jobs 8       # every category + master.md
```

Per-category scripts: `test262-language.py`, `test262-built-ins.py`,
`test262-harness.py`, `test262-intl402.py`, `test262-staging.py`,
`test262-annexB.py`. `test262-all.py` orchestrates them and writes
`docs/test/master.md`.

Common flags (all scripts accept them): `--jobs N`, `--only SUBSTR`
(repeatable), `--timeout-ms N`, `--chunk-timeout S`, `--strict` (non-zero exit
on any failure), `--build`, `--docs DIR`, `--logs DIR`, `--log-all`,
`--no-cache`.

Runner environment variables:

| Variable                | Effect                                                        |
| ----------------------- | ------------------------------------------------------------- |
| `V16_T262_TRACE=1`     | Print `>>> <path>` for every discovered test (used by scripts). |
| `V16_T262_DEBUG=1`     | Dump the full composed source of each test.                   |
| `V16_T262_TIMEOUT_MS`  | Per-test watchdog timeout (external interrupt).               |
| `V16_T262_SKIP_INTL`   | Skip `intl402/`.                                              |

Script environment variables: `V16_TEST262_ROOT`, `V16_TEST262_HARNESS`,
`V16_TEST262_RUNNER`, `V16_TEST262_DOCS`, `V16_TEST262_LOGS`,
`V16_TEST262_CACHE`.

> **Raw logs and the chunk cache live outside the repo** (`~/.cache/v16/...`) by
> default. A full run creates thousands of files; writing them under the
> workspace makes the editor's file watcher thrash. Keep it that way.

### Benchmarks

```bash
./bench-v8.sh     # downloads combined.js, runs qjs/node/boa/v16, prints scores
```

`bench-v8/` is git-ignored. All 8 benchmarks must pass and the score must not
regress (see below).

## Architecture notes

- **Value representation.** `JSValue` is a `u64` using NaN-boxing
  (`src/value.rs`). Small integers, booleans, `undefined`, `null`, symbols,
  BigInts, strings (as interned `Atom`s) and object/function pointers are all
  packed into the quiet-NaN payload. Be careful to distinguish `TAG_INT` from
  double precision.
- **Atoms.** Property names and string values are interned to `u32` atoms
  (`src/runtime/atom.rs`) via `ctx.intern(...)`. Always compare atoms, not
  `String`s, on the hot path.
- **Compiler pipeline.** `lexer → parser (src/compiler/parser.rs) →
  early_errors::validate_program → codegen::CodeGenerator → peephole →
  opcode::Bytecode` (`src/compiler/mod.rs`). `OptLevel` is `O0..O3`.
  `codegen.rs` also has env-tunable micro-optimizations (`V16_OPT_*`,
  `V16_P2_PROFILE`) used for benchmarking; defaults are the committed ones.
- **VM.** `src/runtime/vm.rs` is a large register-based interpreter with a call
  frame stack, exception handlers, inline caches (`src/compiler/ic.rs`), and
  generator/async suspension. Pay attention to handler cleanup on return/throw
  and to `RUST_CALL_DEPTH` (deep recursion must raise `RangeError`, not overflow
  the native stack).
- **GC.** `src/runtime/gc.rs` is a nursery + tenured collector. Newly allocated
  objects/arrays/functions **must be registered** (`gc_heap_mut().track*`) and
  rooted while live. `V16_GCDUMP=1|summary|roots|live|dead|alloc|all` and
  `V16_GCDUMP_LIMIT` dump GC activity.
- **Objects.** Hidden classes via `Shape`/`ShapeCache` (property offset
  transitions) with polymorphic inline caches. Arrays have a dense fast path and
  a sparse fallback (`src/object/array_obj.rs`).
- **Built-ins.** Registered by name in each module's `init_*`, wired up in
  `src/builtins/global.rs::init_globals`. `HostFunction::new` (plain),
  `HostFunction::method` (receives `this`), and `HostFunction::ctor` (callable as
  a constructor) express the different call conventions.
- **Event loop.** `src/runtime/event_loop.rs` provides timers, microtasks
  (promise jobs), macrotasks, and animation callbacks. `eval_async` = `eval` +
  `run_event_loop`. Host I/O is integrated through `MacroTaskExtension`.
- **Modules.** ESM lives in `src/runtime/module.rs`; dynamic `import()` is a
  builtin. Test262 `module`-flagged tests are intentionally skipped.

## Embedding API (summary)

`src/lib.rs` is the public surface. Key entry points:

```rust
use v16::{JSRuntime, JSContext, JSValue, eval, eval_async, OptLevel};

let mut rt = JSRuntime::new();
let mut ctx = rt.new_context();
let v = eval(&mut ctx, "1 + 2").unwrap();
```

Other public helpers: `eval_via_ast`, `eval_with_opt_level`,
`compile_to_bytecode`, `compile_to_register_bytecode`,
`run_event_loop(_with_timeout)`, `eval_async(_with_timeout)`,
`set_interrupt`/`set_external_interrupt`. `ctx.register_global_builtin(name,
arity, fn)` adds a Rust function to JS.

## Conventions

- **The source is deliberately comment-free.** Almost every file has zero
  comments; a handful of the densest modules have a few. Do not add narration
  comments to Rust code — match the surrounding style. (The Python scripts under
  `scripts/` are the exception and are heavily documented.)
- **Names are descriptive and full:** `js_to_length`, `call_function_with_this`,
  `define_property_ext`. Follow that.
- Keep edits scoped and consistent with neighbouring code; avoid opportunistic
  refactors.
- Each logical fix is its own commit. Commit subjects are imperative and terse
  (e.g. `Array.splice: spec deleteCount default, sparse shift, clamp length`),
  usually prefixed by the affected built-in/module.
- Everything in the repository is written in English.

## Non-negotiables

These come from `test262_plan.md` and are the project's hard rules:

1. **No bench-v8 regressions.** All 8 benchmarks must still pass and the score
   must not drop meaningfully (~±5% is noise).
2. **No crashes in Test262.** No category may segfault, abort, or hang — every
   run must complete.
3. **Do not add code comments.**
4. **One fix/optimization per commit.**

Before committing an engine change, run at minimum `cargo test` and the
relevant Test262 category, and regenerate the affected report under `docs/test/`.

## Gotchas

- `tests/readme_examples.rs` is compiled into the test suite. Keep it in sync
  with `README.md`.
- `v16 -compile` output `.jsc` files are version-coupled to the bytecode layout
  in `src/compiler/opcode.rs`; changing opcodes invalidates existing `.jsc`.
- `src/runtime/gc.rs` and the runner assume the whole runtime is dropped between
  Test262 tests; leaks there cause OOM-kill of long runs.
- `test262/`, `target/`, `bench-v8/` and `docs/test/` are excluded from the
  editor's watcher/search (`.vscode/settings.json`) on purpose — do not add them
  to the index.
- The version in `README.md` snippets and `Cargo.toml` drifted in the past;
  keep them in sync.
