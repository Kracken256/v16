# pipa (枇杷) — a fast, minimal ES2023 JavaScript runtime built in Rust

pipa is a from-scratch JavaScript engine: a hand-written lexer and parser, a
register-based bytecode compiler, and a bytecode VM, all implemented in Rust with
essentially **no external C libraries** (the only native code comes from the
optional `rustls`/`ring` TLS backend behind `fetch`). It targets the
**ECMAScript 2023** specification and ships an embeddable library, a CLI, a REPL,
and a built-in networking stack.

```
JavaScript source ──► Lexer ──► Parser ──► Early errors ──► Codegen ──► Peephole
                                                                          │
                                                     .jsc bytecode ◄──────┘
                                                          │
                                                          ▼
                                                       VM + GC
```

- **Spec-focused** — verified against [Test262](https://github.com/tc39/test262)
  (see [docs/test/](docs/test/README.md) for the generated reports).
- **Fast** — competitive with QuickJS on the V8 benchmark suite.
- **Small** — ~5.7 MB release binary with the `repl` feature.
- **Batteries included** — regex, JSON, Base64, BigInt, Unicode, `fetch`,
  WebSocket, Server-Sent Events, and `process`, all implemented from scratch.
- **Embeddable** — use it as a `pipa-js` Rust crate in your own project.

## Table of contents

- [Features](#features)
- [Benchmarks](#benchmarks)
- [Test262 conformance](#test262-conformance)
- [Installation](#installation)
- [Usage](#usage)
- [Embedding in Rust](#embedding-in-rust)
- [Development](#development)
- [Project layout](#project-layout)
- [Documentation](#documentation)
- [License](#license)

## Features

- **ES2023 support** — implements the ECMAScript 2023 specification.
- **Async/await & generators** — first-class `async`/`await`, generators,
  async generators, `for await…of`, and `yield*` without transpilation.
- **Bytecode** — compile JavaScript to `.jsc` files for fast loading, with
  optimization levels `-O0` through `-O3`.
- **Zero-dependency built-ins** — no external C libraries or system packages:

  | Area                  | Highlights                                                       |
  | --------------------- | ---------------------------------------------------------------- |
  | Regex                 | Own parser/compiler with DFA + backtracking engines and optimizers |
  | JSON / Base64 / BigInt | Implemented in Rust                                              |
  | Unicode               | Generated tables (case mapping, identifier classes)              |
  | Networking            | `fetch` (HTTPS via `rustls`), `WebSocket`, `EventSource`         |
  | Host                  | `process.{exec,cwd,exit,argv,env}`, ESM `import()`               |

- **Embeddable runtime** — a small, documented Rust API (`JSRuntime`,
  `JSContext`, `eval`, `eval_async`, bytecode compilation).

## Benchmarks

`./bench-v8.sh` runs the V8 benchmark suite against every engine it finds on
`PATH` (`qjs`, `node`, `boa`) plus `pipa`. A snapshot from **2026-06-15**
(higher is better):

| Benchmark         |   qjs |   node |   boa |  pipa | vs qjs |
| ----------------- | ----: | -----: | ----: | ----: | ------ |
| Richards          |   881 |  44629 |   133 |   866 |  -1.7% |
| DeltaBlue         |   875 |  88373 |   136 |   828 |  -5.4% |
| Crypto            |  1013 |  56374 |   118 |   985 |  -2.8% |
| RayTrace          |  1268 |  70299 |   276 |   589 | -53.5% |
| EarleyBoyer       |  1831 |  84585 |   341 |  1225 | -33.1% |
| RegExp            |   306 |  12316 |  58.1 |   841 | +174.8% |
| Splay             |  2206 |  44299 |   459 |  2000 |  -9.3% |
| NavierStokes      |  1759 |  54166 |   257 |  2087 | +18.6% |
| **SCORE (total)** | **1101** | **50001** | **185** | **1075** | **-2.4%** |

Ranking: **#1 node** · **#2 qjs** · **#3 pipa** · **#4 boa**.

Run it yourself:

```bash
./bench-v8.sh
```

## Test262 conformance

pipa is continuously checked against [tc39/test262](https://github.com/tc39/test262)
(excluding `intl402` compatibility gaps that are tracked separately). The full,
per-test results are generated into [`docs/test/`](docs/test/README.md) by the
scripts in [`scripts/test/`](scripts/test).

Highlights from the latest run:

| Category | Area | Approx. pass rate |
| -------- | ---- | ----------------- |
| Built-ins | `Math`, `Boolean`, `Number`, `parseInt`, `parseFloat`, `Object.*` | ~100% |
| Built-ins | `Symbol`, `Date`, `Function`, `URI` | 85–95% |
| Built-ins | `String`, `Reflect`, `JSON`, `Error` | 50–80% |
| Built-ins | `RegExp`, `BigInt`, `Map`, `Set` | 20–45% |
| Built-ins | `Promise`, `Proxy` | in progress |

See [`docs/test/master.md`](docs/test/master.md) for the aggregated report and
each `docs/test/<category>/master.md` for the per-category breakdown, failure
groups, and the appendix listing every discovered test.

Regenerate the reports with:

```bash
cargo build -r --example test262_runner
python3 scripts/test/test262-all.py --jobs 8
```

## Installation

### As a CLI

```bash
cargo install pipa-js
```

### As a library

```toml
[dependencies]
pipa-js = "0.1.8"
```

If you don't need the REPL, `fetch`, or `process` support, opt out of the
default features to keep the dependency tree small:

```toml
[dependencies]
pipa-js = { version = "0.1.8", default-features = false }
```

### From source

```bash
cargo build --release
./target/release/pipa script.js
```

## Usage

```bash
# Run a script.
pipa script.js

# Run precompiled bytecode.
pipa script.jsc

# Compile JavaScript to bytecode.
pipa -compile input.js output.jsc

# Disassemble bytecode (debugging).
pipa -diss script.jsc

# Specify an optimization level (default: -O2).
pipa -O3 script.js

# Start the REPL (requires the `repl` feature, enabled by default).
pipa
```

The CLI exposes the usual host globals, including `console.log`, `setTimeout`,
`setInterval`, `fetch`, `WebSocket`, `EventSource`, and `import()` for dynamic
ES modules.

## Embedding in Rust

Use pipa-js as a library to embed JavaScript in your Rust project:

```toml
[dependencies]
pipa-js = "0.1.8"
```

### Evaluate JavaScript

```rust
use pipa::{JSRuntime, eval};

let mut rt = JSRuntime::new();
let mut ctx = rt.new_context();

let val = eval(&mut ctx, "1 + 2").unwrap();
assert_eq!(val.get_int(), 3);
```

### Read strings & values from JavaScript

```rust
use pipa::{JSRuntime, eval};

let mut rt = JSRuntime::new();
let mut ctx = rt.new_context();

eval(&mut ctx, r#"
    function greet(name) {
        return "Hello, " + name + "!";
    }
"#).unwrap();

let val = eval(&mut ctx, r#"greet("world")"#).unwrap();
assert!(val.is_string());
let s = ctx.get_atom_str(val.get_atom());
assert_eq!(s, "Hello, world!");
```

### Call custom Rust functions from JavaScript

```rust
use pipa::{JSRuntime, eval, JSValue};

fn js_print(ctx: &mut pipa::JSContext, args: &[JSValue]) -> JSValue {
    for arg in args {
        if arg.is_string() {
            print!("{}", ctx.get_atom_str(arg.get_atom()));
        } else if arg.is_int() {
            print!("{}", arg.get_int());
        }
    }
    println!();
    JSValue::undefined()
}

let mut rt = JSRuntime::new();
let mut ctx = rt.new_context();

ctx.register_global_builtin("print", 1, js_print);
eval(&mut ctx, r#"print("hello from Rust!")"#).unwrap();
```

### Async/await with an event loop

```rust
use pipa::{JSRuntime, eval, eval_async};

let mut rt = JSRuntime::new();
let mut ctx = rt.new_context();

eval_async(&mut ctx, r#"
    var result = null;
    (async () => {
        result = await fetch("https://httpbin.org/json");
    })();
"#).unwrap();

let val = eval(&mut ctx, "JSON.stringify(result)").unwrap();
println!("{}", ctx.get_atom_str(val.get_atom()));
```

> Requires the `fetch` feature (enabled by default). `eval_async` is `eval` +
> `run_event_loop` in one call; use `run_event_loop_with_timeout` to bound the
> loop.

### Bytecode compilation

```rust
use pipa::{JSRuntime, eval, compile_to_register_bytecode};

let mut rt = JSRuntime::new();
let mut ctx = rt.new_context();

// Compile JavaScript to register-based bytecode
let (code, constants) = compile_to_register_bytecode(
    &mut ctx,
    "function fib(n) { return n < 2 ? n : fib(n-1) + fib(n-2); } fib(20)",
).unwrap();

// code: Vec<u8>, constants: Vec<JSValue>
assert!(!code.is_empty());
```

The `tests/readme_examples.rs` integration test mirrors these snippets, so keep
them in sync when you change the API.

## Development

### Build

```bash
# Default build: REPL + fetch + process.
cargo build --release

# Minimal engine.
cargo build --release --no-default-features
```

Feature flags:

| Feature              | Default | Enables                                              |
| -------------------- | ------- | ---------------------------------------------------- |
| `repl`               | ✅      | `fetch` + `process` + the `rustyline` REPL            |
| `fetch`              | ✅\*    | `src/http/`, `fetch`, `WebSocket`, `EventSource`      |
| `process`            | ✅\*    | `process.exec/cwd/exit` built-ins                    |
| `full_runtime_tests` | ❌      | Compiles the in-process Test262 harness test         |

\* pulled in by `repl`, which is on by default.

### Test

```bash
cargo test                                    # unit + integration tests
cargo test --features full_runtime_tests      # adds the Test262 harness
```

### Test262

```bash
./test262.sh                                  # clone + run the whole suite
python3 scripts/test/test262-all.py --jobs 8  # run + regenerate docs/test/
```

See [`docs/test/README.md`](docs/test/README.md) for the report format and all
flags (`--only`, `--strict`, `--no-cache`, …).

### Benchmarks

```bash
./bench-v8.sh
```

## Project layout

| Path            | Contents                                                            |
| --------------- | ------------------------------------------------------------------- |
| `src/compiler/` | Lexer, parser, early errors, codegen, peephole optimizer, opcodes.  |
| `src/runtime/`  | `JSRuntime`/`JSContext`, VM, GC, event loop, modules.               |
| `src/object/`   | Objects, functions, arrays, hidden-class shapes, inline caches.     |
| `src/builtins/` | The standard library, plus `global.rs` for the global environment.  |
| `src/regexp/`   | Standalone regular-expression engine.                               |
| `src/http/`     | HTTP client, TLS glue, WebSocket.                                   |
| `src/host/`     | Embedding types (`HostFunction`, `ToJS`, `FromJS`).                 |
| `tests/`        | Rust integration tests.                                             |
| `examples/`     | Examples and the Test262 runner.                                    |
| `scripts/test/` | Python drivers that run Test262 and write reports.                  |
| `docs/test/`    | Generated conformance reports (checked in).                         |

See [AGENTS.md](AGENTS.md) for a deeper architecture guide and the project's
contribution rules.

## Documentation

- [AGENTS.md](AGENTS.md) — architecture, conventions, and contributor rules.
- [docs/test/README.md](docs/test/README.md) — Test262 report format and usage.
- [test262_plan.md](test262_plan.md) — historical engineering log.

## License

MIT
