#!/usr/bin/env python3
"""Shared engine for running Test262 categories and producing Markdown reports.

The Rust example binary ``examples/test262_runner.rs`` only prints a ``✓`` line
for the first 50 passing tests (and every 500th afterwards) and only prints the
full error detail for the first 5 failures.  That makes its plain output
useless for per-test reporting.

The runner does, however, support ``V16_T262_TRACE=1`` which prints a
``>>> <path>`` line for *every* test file it discovers (including tests that
are later skipped).  Combined with the fact that every failure prints a
``✗ <path> - <error>`` line, the trace output is enough to reconstruct the
exact status of every single test:

* ``>>> path``      -> the test was discovered and started
* ``✗ path - msg``  -> the test failed with ``msg``
* ``✓ path``        -> the test passed (only printed for the first 50)
* no ``✗`` line      -> the test passed (when the run completed normally)

Skipped tests are identified in Python by re-implementing the runner's
``should_skip`` logic against the YAML frontmatter, so that skipped tests are
reported as ``Skipped`` rather than ``Passed``.

Typical usage::

    python3 scripts/test/test262-language.py
    python3 scripts/test/test262-all.py --jobs 12

Every category script writes ``docs/test/<category>/master.md`` and the
orchestrator writes ``docs/test/master.md``.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import dataclasses
import datetime as _datetime
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time
from collections import Counter, OrderedDict
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Tuple

try:  # PyYAML is used to read the test262 frontmatter.
    import yaml
except ImportError:  # pragma: no cover - fallback so the scripts still start
    yaml = None


# ---------------------------------------------------------------------------
# Paths and constants
# ---------------------------------------------------------------------------

REPO_ROOT = Path(__file__).resolve().parents[2]
TEST262_ROOT = Path(os.environ.get("V16_TEST262_ROOT", REPO_ROOT / "test262" / "test"))
HARNESS_DIR = Path(os.environ.get("V16_TEST262_HARNESS", REPO_ROOT / "test262" / "harness"))
RUNNER_BIN = Path(
    os.environ.get(
        "V16_TEST262_RUNNER",
        REPO_ROOT / "target" / "release" / "examples" / "test262_runner",
    )
)
DOCS_ROOT = Path(os.environ.get("V16_TEST262_DOCS", REPO_ROOT / "docs" / "test"))

#: Raw runner logs deliberately live *outside* the repository.  A full run
#: creates thousands of log files, and writing them into the workspace makes
#: VS Code's file watcher and extension host thrash (which crashes the editor).
CACHE_ROOT = Path(
    os.environ.get(
        "XDG_CACHE_HOME",
        Path.home() / ".cache",
    )
)
LOG_ROOT = Path(os.environ.get("V16_TEST262_LOGS", CACHE_ROOT / "v16" / "test262-logs"))

#: Chunk results are cached here so that an interrupted run resumes instead of
#: starting over.  A full language run takes long enough that a crashed editor
#: (or a closed terminal) would otherwise throw away all of the finished work.
#: The cache lives outside the repository for the same reason as the logs.
CHUNK_CACHE_ROOT = Path(
    os.environ.get("V16_TEST262_CACHE", CACHE_ROOT / "v16" / "test262-cache")
)

#: Bump this whenever the log parsing or the pass/fail classification changes.
#: It is mixed into every cache key, so a bump automatically invalidates all
#: previously cached results instead of silently reusing stale verdicts.
CACHE_VERSION = 1

#: Categories that exist under ``test262/test`` and are driven by the scripts.
CATEGORIES: Tuple[str, ...] = (
    "language",
    "built-ins",
    "harness",
    "intl402",
    "staging",
    "annexB",
)

#: Test files the Rust runner skips unconditionally (``collect_tests``).
EXCLUDED_FILENAMES = frozenset(
    {
        "RegExp-leading-escape-BMP.js",
        "proto-from-ctor-realm.js",
        "Math.hypot_ToNumberErr.js",
        "Math.max_each-element-coerced.js",
        "Math.min_each-element-coerced.js",
        "15.4.4.19-3-28.js",
        "15.4.4.19-3-29.js",
        "15.4.4.15-3-28.js",
        "15.4.4.16-3-29.js",
        "15.4.4.14-3-28.js",
        "15.4.4.14-3-29.js",
        "15.4.4.17-3-28.js",
        "15.4.4.17-3-29.js",
        "asyncitems-arraylike-too-long.js",
        "cause_abrupt.js",
        "this-val-non-symbol.js",
    }
)

#: ``flags`` entries that make the runner skip a test (``should_skip``).
UNSUPPORTED_FLAGS = ("module", "raw")

#: Default per-test timeout passed to the runner (milliseconds).
DEFAULT_TEST_TIMEOUT_MS = 10_000
#: Per-category watchdog overrides.  ``harness/`` holds support-library files
#: rather than real tests; they never call ``$DONE()``, so the watchdog is the
#: only thing that ends them.  A short timeout keeps that category quick.
CATEGORY_TIMEOUT_MS = {"harness": 2_000}
#: Default wall-clock timeout for one chunk subprocess (seconds).
DEFAULT_CHUNK_TIMEOUT_S = 3_600
#: Default number of chunk subprocesses to run in parallel.  Kept deliberately
#: modest: each worker is a full VM process, and saturating every core makes an
#: editor running alongside the suite unresponsive.  ``--jobs`` overrides it.
DEFAULT_JOBS = max(2, min(4, (os.cpu_count() or 4) // 4))
#: ``nice`` increment applied to every runner subprocess.  The suite is a batch
#: job; an interactive editor must always win the CPU scheduler.
NICE_LEVEL = 10
#: Maximum number of characters kept for a test name in the appendix table.
#: Names longer than this are elided in the middle so the file name survives.
MAX_TEST_NAME_CHARS = 96
#: Maximum number of characters kept for an error message in a table cell.
MAX_MESSAGE_CHARS = 240

STATUS_PASSED = "Passed"
STATUS_FAILED = "Failed"
STATUS_SKIPPED = "Skipped"
STATUS_NOT_RUN = "Not run"
STATUS_UNKNOWN = "Unknown"

STATUS_ORDER = (
    STATUS_PASSED,
    STATUS_FAILED,
    STATUS_SKIPPED,
    STATUS_NOT_RUN,
    STATUS_UNKNOWN,
)

# ---------------------------------------------------------------------------
# Data structures
# ---------------------------------------------------------------------------


@dataclasses.dataclass
class TestResult:
    """One row of the appendix table."""

    path: str  # posix path relative to TEST262_ROOT
    status: str
    message: str = ""
    notes: str = ""

    @property
    def failed(self) -> bool:
        return self.status == STATUS_FAILED


@dataclasses.dataclass
class ChunkResult:
    """Result of running one chunk (a subdirectory or a single .js file)."""

    chunk: str  # posix path relative to TEST262_ROOT
    results: List[TestResult] = dataclasses.field(default_factory=list)
    summary: Optional[Dict[str, int]] = None
    duration: float = 0.0
    returncode: int = 0
    timed_out: bool = False
    finished: bool = False
    log_path: Optional[Path] = None
    warnings: List[str] = dataclasses.field(default_factory=list)
    #: True when the result was served from the on-disk chunk cache instead of
    #: being produced by a fresh runner invocation.
    cached: bool = False


@dataclasses.dataclass
class CategoryResult:
    """Aggregated result for one category."""

    category: str
    results: List[TestResult] = dataclasses.field(default_factory=list)
    duration: float = 0.0
    jobs: int = DEFAULT_JOBS
    warnings: List[str] = dataclasses.field(default_factory=list)
    chunks: int = 0
    generated_at: str = ""
    runner: str = ""
    command: str = ""

    def counts(self) -> Dict[str, int]:
        counter = Counter(result.status for result in self.results)
        return {status: counter.get(status, 0) for status in STATUS_ORDER}


# ---------------------------------------------------------------------------
# Test discovery (mirrors examples/test262_runner.rs::collect_tests)
# ---------------------------------------------------------------------------


def _is_test_file(path: Path) -> bool:
    name = path.name
    if name.endswith("_FIXTURE.js") or name.endswith("_templates.js"):
        return False
    if name in ("assert.js", "sta.js"):
        return False
    if name in EXCLUDED_FILENAMES:
        return False
    return True


def discover_test_files(root: Path) -> List[Path]:
    """Return every test file under ``root`` in the same order the runner uses.

    The runner walks directories and then sorts the collected files by their
    slash-prefixed relative path, so we sort by the same key to keep the
    appendix stable across runs.
    """

    if root.is_file():
        return [root] if root.suffix == ".js" and _is_test_file(root) else []

    files: List[Path] = []
    for path in root.rglob("*.js"):
        if path.is_file() and _is_test_file(path):
            files.append(path)
    files.sort(key=lambda p: "/" + p.relative_to(root).as_posix())
    return files


def category_chunks(category_dir: Path) -> List[Path]:
    """Split a category into independently runnable chunks.

    A category is split two levels deep so that big categories such as
    ``language`` and ``built-ins`` load-balance across the parallel workers:

    * a top-level ``.js`` file is its own chunk;
    * for a top-level directory, each of its subdirectories is a chunk and each
      of its directly-contained ``.js`` files is its own chunk;
    * a top-level directory with no subdirectories is one chunk.

    Chunks never overlap, so every test file is executed exactly once.
    """

    if not category_dir.exists():
        return []
    if category_dir.is_file():
        return [category_dir]

    chunks: List[Path] = []
    for entry in sorted(category_dir.iterdir(), key=lambda p: p.name):
        if entry.is_file():
            if entry.suffix == ".js" and _is_test_file(entry):
                chunks.append(entry)
            continue
        if not entry.is_dir():
            continue

        subdirectories: List[Path] = []
        direct_files: List[Path] = []
        for child in sorted(entry.iterdir(), key=lambda p: p.name):
            if child.is_dir():
                subdirectories.append(child)
            elif child.suffix == ".js" and _is_test_file(child):
                direct_files.append(child)

        if subdirectories:
            chunks.extend(subdirectories)
            chunks.extend(direct_files)
        else:
            chunks.append(entry)
    return chunks


# ---------------------------------------------------------------------------
# Frontmatter parsing (mirrors examples/test262_runner.rs::parse_frontmatter)
# ---------------------------------------------------------------------------


def read_source(path: Path) -> Optional[str]:
    try:
        return path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None


def split_frontmatter(text: str) -> Optional[Tuple[str, str]]:
    """Return ``(yaml_block, code)`` for a test262 source file."""

    if text.startswith("/*---"):
        start = 0
    else:
        marker = text.find("\n/*---")
        if marker < 0:
            return None
        start = marker + 1

    yaml_start = start + 5
    end = text.find("---*/", yaml_start)
    if end < 0:
        return None

    block = text[yaml_start:end]
    # Some files carry an explicit YAML document separator just before the
    # closing marker; strip it like the Rust implementation does.
    separator = block.rfind("\n---")
    if separator >= 0:
        block = block[:separator]

    return block, text[end + 5 :]


def parse_frontmatter(path: Path) -> Optional[Dict[str, Any]]:
    """Parse the YAML frontmatter of a test file, or ``None`` if unreadable."""

    text = read_source(path)
    if text is None:
        return None
    split = split_frontmatter(text)
    if split is None:
        return None
    block, _code = split
    if yaml is None:
        return {}
    try:
        meta = yaml.safe_load(block)
    except Exception:
        return None
    if not isinstance(meta, dict):
        return {}
    return meta


def _as_str_list(value: Any) -> List[str]:
    if not isinstance(value, (list, tuple)):
        return []
    return [item for item in value if isinstance(item, str)]


def skip_reasons(meta: Optional[Dict[str, Any]]) -> List[str]:
    """Return the reasons the runner would skip this test (empty == run it)."""

    if not meta:
        return []

    reasons: List[str] = []
    flags = _as_str_list(meta.get("flags"))
    for flag in UNSUPPORTED_FLAGS:
        if flag in flags:
            reasons.append("flag: " + flag)

    return reasons


def is_skipped(meta: Optional[Dict[str, Any]]) -> bool:
    return bool(skip_reasons(meta))


# ---------------------------------------------------------------------------
# Runner output parsing
# ---------------------------------------------------------------------------

TRACE_RE = re.compile(r"^>>> (.*)$")
FAIL_RE = re.compile(r"^\s*✗\s?(.*)$")
PASS_RE = re.compile(r"^\s*✓\s?(.*)$")
DETAIL_RE = re.compile(r"^\s{4}\|\s?(.*)$")
SUMMARY_RE = re.compile(
    r"^Results:\s+(\d+) passed,\s+(\d+) failed,\s+(\d+) skipped of (\d+) total"
)
MODE_LABEL_RE = re.compile(r"^\s*\((?:non-strict|strict)\)\s*")


def _extract_error(rest: str, current: str) -> str:
    """Pull the error text out of the part of a ``✗`` line after the glyph."""

    rest = rest.strip()
    if current and rest.startswith(current):
        tail = rest[len(current) :]
        tail = MODE_LABEL_RE.sub("", tail).strip()
        if tail.startswith("-"):
            tail = tail[1:].strip()
        return tail
    # Fallback: split on the first " - " separator.
    if " - " in rest:
        return rest.split(" - ", 1)[1].strip()
    return rest


def _trace_to_relative(chunk: Path, trace_path: str) -> str:
    """Map a ``>>> /path`` trace entry back to a path relative to TEST262_ROOT."""

    if chunk.is_file():
        try:
            return chunk.relative_to(TEST262_ROOT).as_posix()
        except ValueError:
            return chunk.as_posix()

    base = chunk.relative_to(TEST262_ROOT).as_posix() if chunk != TEST262_ROOT else ""
    relative = trace_path.lstrip("/")
    return f"{base}/{relative}" if base else relative


def parse_runner_log(text: str, chunk: Path) -> Tuple[Dict[str, Dict[str, Any]], List[str], Optional[Dict[str, int]], bool]:
    """Parse the trace output of one runner invocation.

    Returns ``(records, order, summary, finished)`` where ``records`` maps a
    path relative to ``TEST262_ROOT`` to ``{"status", "message", "details"}``.
    """

    records: "OrderedDict[str, Dict[str, Any]]" = OrderedDict()
    order: List[str] = []
    summary: Optional[Dict[str, int]] = None
    finished = False
    current: Optional[str] = None

    for raw_line in text.splitlines():
        line = raw_line.rstrip("\r")

        trace = TRACE_RE.match(line)
        if trace:
            relative = _trace_to_relative(chunk, trace.group(1).strip())
            current = relative
            if relative not in records:
                order.append(relative)
            # A repeated trace entry keeps the newest state.
            records[relative] = {"status": STATUS_PASSED, "message": "", "details": []}
            continue

        if SUMMARY_RE.match(line):
            match = SUMMARY_RE.match(line)
            assert match is not None
            summary = {
                "passed": int(match.group(1)),
                "failed": int(match.group(2)),
                "skipped": int(match.group(3)),
                "total": int(match.group(4)),
            }
            finished = True
            current = None
            continue

        fail = FAIL_RE.match(line)
        if fail and current is not None:
            record = records[current]
            record["status"] = STATUS_FAILED
            record["message"] = _extract_error(fail.group(1), current)
            continue

        detail = DETAIL_RE.match(line)
        if detail and current is not None:
            record = records[current]
            if record["status"] == STATUS_FAILED:
                record["details"].append(detail.group(1))
            continue

        passed = PASS_RE.match(line)
        if passed and current is not None:
            records[current]["status"] = STATUS_PASSED
            continue

    return records, order, summary, finished


# ---------------------------------------------------------------------------
# Running one chunk
# ---------------------------------------------------------------------------


def _safe_log_name(chunk: Path) -> str:
    try:
        relative = chunk.relative_to(TEST262_ROOT).as_posix()
    except ValueError:
        relative = chunk.as_posix()
    return relative.replace("/", "__") + ".log"


def run_chunk(
    chunk: Path,
    *,
    timeout_ms: int = DEFAULT_TEST_TIMEOUT_MS,
    chunk_timeout: int = DEFAULT_CHUNK_TIMEOUT_S,
    log_dir: Optional[Path] = None,
    extra_env: Optional[Dict[str, str]] = None,
    log_all: bool = False,
) -> ChunkResult:
    """Run the Rust test262 runner on a single chunk and classify every test.

    When ``log_dir`` is set, the raw output is saved for every chunk that has a
    failure, a not-run test or a warning.  Pass ``log_all=True`` to keep the log
    for passing chunks too — useful for debugging, but it creates thousands of
    files per run.
    """

    try:
        chunk_relative = chunk.relative_to(TEST262_ROOT).as_posix()
    except ValueError:
        chunk_relative = chunk.as_posix()

    result = ChunkResult(chunk=chunk_relative)

    # Files the runner would discover for this chunk, computed up front so that
    # tests that never got a chance to run are still reported.
    discovered = discover_test_files(chunk)
    discovered_relative = [path.relative_to(TEST262_ROOT).as_posix() for path in discovered]

    env = os.environ.copy()
    env["V16_T262_TRACE"] = "1"
    env["V16_T262_TIMEOUT_MS"] = str(timeout_ms)
    if extra_env:
        env.update(extra_env)

    # Run the runner at a lower scheduling priority so a batch run never makes
    # an interactive editor (or anything else) unresponsive.  ``nice`` is used
    # as a wrapper instead of ``preexec_fn`` because the latter is unsafe when
    # called from a thread pool.
    command = [str(RUNNER_BIN), str(chunk), str(HARNESS_DIR)]
    if NICE_LEVEL > 0 and shutil.which("nice"):
        command = ["nice", "-n", str(NICE_LEVEL)] + command

    start = time.monotonic()
    text = ""
    try:
        completed = subprocess.run(
            command,
            cwd=str(REPO_ROOT),
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=chunk_timeout,
        )
        text = completed.stdout.decode("utf-8", "replace") if completed.stdout else ""
        result.returncode = completed.returncode
    except subprocess.TimeoutExpired as exc:
        result.timed_out = True
        result.returncode = -9
        partial = exc.stdout or b""
        if isinstance(partial, str):
            text = partial
        else:
            text = partial.decode("utf-8", "replace")
    except FileNotFoundError:
        result.returncode = -1
        result.warnings.append(
            "runner binary not found at %s (build it with "
            "`cargo build -r --example test262_runner`)" % RUNNER_BIN
        )
    finally:
        result.duration = time.monotonic() - start

    records, order, summary, finished = parse_runner_log(text, chunk)
    result.summary = summary
    result.finished = finished

    seen = set(order)
    for relative in order:
        record = records[relative]
        path = TEST262_ROOT / relative
        meta = parse_frontmatter(path)
        reasons = skip_reasons(meta)
        if record["status"] == STATUS_FAILED:
            message = record["message"]
            details = record.get("details") or []
            if details:
                message = message + " | " + " ".join(details)
            result.results.append(TestResult(relative, STATUS_FAILED, message))
        elif reasons:
            result.results.append(TestResult(relative, STATUS_SKIPPED, "", "; ".join(reasons)))
        else:
            result.results.append(TestResult(relative, STATUS_PASSED))

    # Any discovered file that never produced a trace line was never reached.
    for relative in discovered_relative:
        if relative in seen:
            continue
        if result.timed_out:
            note = "chunk timed out after %ss" % chunk_timeout
        elif not finished:
            note = "runner exited before this test was reached"
        else:
            note = "not discovered by the runner"
        result.results.append(TestResult(relative, STATUS_NOT_RUN, "", note))

    if not finished:
        if result.timed_out:
            result.warnings.append(
                "chunk timed out after %ss; %d test(s) not reached" % (chunk_timeout, len(discovered_relative) - len(seen))
            )
        elif result.returncode != 0:
            result.warnings.append(
                "runner exited with code %d before printing the summary" % result.returncode
            )
        else:
            result.warnings.append("runner output had no summary line")

    if summary is not None:
        computed_skipped = sum(1 for item in result.results if item.status == STATUS_SKIPPED)
        if computed_skipped != summary.get("skipped", 0):
            result.warnings.append(
                "skip count mismatch: runner skipped %d, frontmatter says %d"
                % (summary.get("skipped", 0), computed_skipped)
            )
        computed_failed = sum(1 for item in result.results if item.status == STATUS_FAILED)
        if computed_failed != summary.get("failed", 0):
            result.warnings.append(
                "failure count mismatch: runner failed %d, parsed %d"
                % (summary.get("failed", 0), computed_failed)
            )

    result.results.sort(key=lambda item: item.path)

    # Persist the raw output only when it is likely to be needed.  Keeping every
    # log would create thousands of files per run, which is hostile to editors
    # watching the workspace.
    interesting = log_all or bool(result.warnings) or any(
        item.status in (STATUS_FAILED, STATUS_NOT_RUN, STATUS_UNKNOWN) for item in result.results
    )
    if log_dir is not None and interesting:
        try:
            log_dir.mkdir(parents=True, exist_ok=True)
            log_path = log_dir / _safe_log_name(chunk)
            log_path.write_text(text, encoding="utf-8", errors="replace")
            result.log_path = log_path
        except OSError as exc:  # pragma: no cover - disk problems only
            result.warnings.append("could not write log: %s" % exc)

    return result


# ---------------------------------------------------------------------------
# Chunk result cache (resumability)
# ---------------------------------------------------------------------------


def _chunk_fingerprint(chunk: Path, timeout_ms: int) -> str:
    """Return a stable hash describing *what would be executed* for ``chunk``.

    The fingerprint covers the chunk path, the runner binary's identity, the
    per-test timeout and the size/mtime of every test file the chunk contains.
    A cached result is only reused when all of those are unchanged, so editing
    a test file, rebuilding the runner or changing the timeout always forces a
    fresh run.
    """

    digest = hashlib.sha1()
    digest.update(
        ("v%d|%s|%d" % (CACHE_VERSION, chunk.as_posix(), timeout_ms)).encode("utf-8")
    )
    try:
        stat = RUNNER_BIN.stat()
        digest.update(("|runner:%d:%d" % (stat.st_size, stat.st_mtime_ns)).encode("utf-8"))
    except OSError:
        digest.update(b"|runner:missing")
    for path in discover_test_files(chunk):
        try:
            stat = path.stat()
        except OSError:
            continue
        digest.update(
            ("|%s:%d:%d" % (path.as_posix(), stat.st_size, stat.st_mtime_ns)).encode("utf-8")
        )
    return digest.hexdigest()


def _load_cached_chunk(cache_dir: Path, key: str) -> Optional[ChunkResult]:
    """Return the cached :class:`ChunkResult` for ``key``, or ``None``."""

    path = cache_dir / (key + ".json")
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    try:
        results = [
            TestResult(
                item["path"],
                item["status"],
                item.get("message", ""),
                item.get("notes", ""),
            )
            for item in payload["results"]
        ]
    except (KeyError, TypeError):
        return None
    cached = ChunkResult(
        chunk=payload.get("chunk", ""),
        results=results,
        summary=payload.get("summary"),
        duration=float(payload.get("duration", 0.0)),
        returncode=int(payload.get("returncode", 0)),
        finished=bool(payload.get("finished", False)),
        warnings=list(payload.get("warnings") or []),
        cached=True,
    )
    return cached


def _save_cached_chunk(cache_dir: Path, key: str, result: ChunkResult) -> None:
    """Persist ``result`` so a later run can skip this chunk."""

    payload = {
        "chunk": result.chunk,
        "summary": result.summary,
        "duration": result.duration,
        "returncode": result.returncode,
        "finished": result.finished,
        "warnings": list(result.warnings),
        "results": [
            {
                "path": item.path,
                "status": item.status,
                "message": item.message,
                "notes": item.notes,
            }
            for item in result.results
        ],
    }
    try:
        cache_dir.mkdir(parents=True, exist_ok=True)
        temporary = cache_dir / (key + ".json.tmp")
        temporary.write_text(json.dumps(payload), encoding="utf-8")
        temporary.replace(cache_dir / (key + ".json"))
    except OSError:  # pragma: no cover - disk problems only
        pass


def _chunk_is_cacheable(result: ChunkResult) -> bool:
    """Only complete, failure-free chunks are cached.

    Failures are deliberately never cached: after a fix the runner should
    re-execute exactly the chunks that were failing before.  That makes a
    resumed run converge on the real remaining failures instead of replaying
    yesterday's verdicts.
    """

    if not result.finished or result.timed_out:
        return False
    return all(
        item.status in (STATUS_PASSED, STATUS_SKIPPED) for item in result.results
    )


# ---------------------------------------------------------------------------
# Running a whole category
# ---------------------------------------------------------------------------


def _progress(message: str) -> None:
    print(message, file=sys.stderr, flush=True)


def _chunk_matches(chunk: Path, category_dir: Path, filters: Sequence[str]) -> bool:
    """Return True when ``chunk`` matches any of the ``--only`` filters.

    Both the category-relative and the repository-relative spellings are
    accepted, so ``--only block-scope/syntax`` and ``--only language/block-scope``
    both work.
    """

    candidates = [chunk.as_posix()]
    try:
        candidates.append(chunk.relative_to(category_dir).as_posix())
    except ValueError:
        pass
    try:
        candidates.append(chunk.relative_to(TEST262_ROOT).as_posix())
    except ValueError:
        pass
    return any(needle in candidate for needle in filters for candidate in candidates)


def run_category(
    category: str,
    *,
    jobs: int = DEFAULT_JOBS,
    timeout_ms: Optional[int] = None,
    chunk_timeout: int = DEFAULT_CHUNK_TIMEOUT_S,
    log_dir: Optional[Path] = None,
    only: Optional[Sequence[str]] = None,
    quiet: bool = False,
    log_all: bool = False,
    cache_dir: Optional[Path] = None,
) -> CategoryResult:
    """Run every chunk of ``category`` (in parallel) and merge the results.

    ``timeout_ms`` defaults to :data:`CATEGORY_TIMEOUT_MS` for the category, or
    :data:`DEFAULT_TEST_TIMEOUT_MS` when the category has no override.

    When ``cache_dir`` is given, chunks that previously completed without any
    failure are served from the on-disk cache instead of being re-executed.
    Chunks containing failures are always re-run, so an interrupted run resumes
    exactly where it left off and a fixed bug is verified immediately.
    """

    if timeout_ms is None:
        timeout_ms = CATEGORY_TIMEOUT_MS.get(category, DEFAULT_TEST_TIMEOUT_MS)

    category_dir = TEST262_ROOT / category
    result = CategoryResult(category=category, jobs=jobs)
    result.generated_at = _datetime.datetime.now().astimezone().strftime("%Y-%m-%d %H:%M:%S %Z")
    result.runner = str(RUNNER_BIN)
    result.command = "python3 " + " ".join(sys.argv)

    if not category_dir.exists():
        result.warnings.append("category directory not found: %s" % category_dir)
        return result

    chunks = category_chunks(category_dir)
    if only:
        filters = [item for item in only if item]
        chunks = [chunk for chunk in chunks if _chunk_matches(chunk, category_dir, filters)]
    result.chunks = len(chunks)
    if not chunks:
        result.warnings.append("no chunks found for category %s" % category)
        return result

    category_log_dir = (log_dir / category) if log_dir is not None else None
    if category_log_dir is not None:
        category_log_dir.mkdir(parents=True, exist_ok=True)

    started = time.monotonic()
    completed = 0
    merged: List[TestResult] = []
    worker_count = max(1, min(jobs, len(chunks)))

    def _run_or_reuse(chunk: Path) -> ChunkResult:
        """Run one chunk, or replay its cached result when it already passed.

        ``_chunk_fingerprint`` changes as soon as the runner binary, the timeout
        or any file in the chunk changes, so a cached entry can never mask a
        rebuild or a test edit.
        """

        if cache_dir is None:
            return run_chunk(
                chunk,
                timeout_ms=timeout_ms,
                chunk_timeout=chunk_timeout,
                log_dir=category_log_dir,
                log_all=log_all,
            )
        key = _chunk_fingerprint(chunk, timeout_ms)
        cached = _load_cached_chunk(cache_dir, key)
        if cached is not None:
            return cached
        fresh = run_chunk(
            chunk,
            timeout_ms=timeout_ms,
            chunk_timeout=chunk_timeout,
            log_dir=category_log_dir,
            log_all=log_all,
        )
        if _chunk_is_cacheable(fresh):
            _save_cached_chunk(cache_dir, key, fresh)
        return fresh

    with concurrent.futures.ThreadPoolExecutor(max_workers=worker_count) as pool:
        futures = {pool.submit(_run_or_reuse, chunk): chunk for chunk in chunks}
        for future in concurrent.futures.as_completed(futures):
            chunk = futures[future]
            completed += 1
            try:
                chunk_result = future.result()
            except Exception as exc:  # pragma: no cover - defensive
                relative = chunk.relative_to(TEST262_ROOT).as_posix()
                result.warnings.append("chunk %s raised %s" % (relative, exc))
                continue

            merged.extend(chunk_result.results)
            result.warnings.extend(chunk_result.warnings)
            if not quiet:
                counts = Counter(item.status for item in chunk_result.results)
                _progress(
                    "[%d/%d] %s -> %d passed, %d failed, %d skipped, %d not run (%.1fs)%s"
                    % (
                        completed,
                        len(chunks),
                        chunk_result.chunk,
                        counts.get(STATUS_PASSED, 0),
                        counts.get(STATUS_FAILED, 0),
                        counts.get(STATUS_SKIPPED, 0),
                        counts.get(STATUS_NOT_RUN, 0) + counts.get(STATUS_UNKNOWN, 0),
                        chunk_result.duration,
                        " [cached]" if chunk_result.cached else "",
                    )
                )

    # Deduplicate by path, preferring the most severe status.
    severity = {
        STATUS_FAILED: 5,
        STATUS_UNKNOWN: 4,
        STATUS_NOT_RUN: 3,
        STATUS_PASSED: 2,
        STATUS_SKIPPED: 1,
    }
    deduped: "OrderedDict[str, TestResult]" = OrderedDict()
    for item in merged:
        existing = deduped.get(item.path)
        if existing is None or severity.get(item.status, 0) > severity.get(existing.status, 0):
            deduped[item.path] = item

    result.results = sorted(deduped.values(), key=lambda item: item.path)
    result.duration = time.monotonic() - started
    return result


# ---------------------------------------------------------------------------
# Markdown report rendering
# ---------------------------------------------------------------------------

STATUS_BADGE = {
    STATUS_PASSED: "✅ pass",
    STATUS_FAILED: "❌ fail",
    STATUS_SKIPPED: "⏭️ skip",
    STATUS_NOT_RUN: "⚠️ not run",
    STATUS_UNKNOWN: "❓ unknown",
}

#: Order used when rendering summary tables (problems first).
SUMMARY_ORDER = (STATUS_FAILED, STATUS_NOT_RUN, STATUS_UNKNOWN, STATUS_SKIPPED, STATUS_PASSED)


def _md_escape(text: str) -> str:
    """Escape text for safe use inside a Markdown table cell."""

    if not text:
        return ""
    text = text.replace("\\", "\\\\").replace("|", "\\|")
    text = text.replace("\r", " ").replace("\n", " ").replace("\t", " ")
    text = re.sub(r"\s+", " ", text).strip()
    return text


def _truncate_middle(text: str, limit: int) -> str:
    if len(text) <= limit:
        return text
    head = limit * 2 // 3
    tail = limit - head - 3
    return text[:head] + "..." + text[len(text) - tail :]


def _truncate_path(path: str, prefix: str) -> str:
    """Strip the category prefix from a test path for the appendix table."""

    if prefix and path.startswith(prefix):
        return path[len(prefix) :]
    return path


def _fmt_duration(seconds: float) -> str:
    if seconds < 1:
        return "%.0f ms" % (seconds * 1000)
    if seconds < 60:
        return "%.2f s" % seconds
    minutes, secs = divmod(seconds, 60)
    if minutes < 60:
        return "%d m %02.0f s" % (minutes, secs)
    hours, minutes = divmod(int(minutes), 60)
    return "%d h %02d m" % (hours, minutes)


def _summarize(category: CategoryResult) -> "OrderedDict[str, int]":
    counts: "OrderedDict[str, int]" = OrderedDict((status, 0) for status in SUMMARY_ORDER)
    for item in category.results:
        counts[item.status] = counts.get(item.status, 0) + 1
    return counts


def _group_failures(category: CategoryResult, limit: int = 40) -> "OrderedDict[str, List[TestResult]]":
    """Group failures by their error signature so repeated causes collapse."""

    groups: "OrderedDict[str, List[TestResult]]" = OrderedDict()
    for item in category.results:
        if item.status != STATUS_FAILED:
            continue
        signature = item.message.strip() or "(no message)"
        signature = _truncate_middle(signature, 160)
        groups.setdefault(signature, []).append(item)
    ordered = sorted(groups.items(), key=lambda kv: (-len(kv[1]), kv[0]))
    return OrderedDict(ordered[:limit])


def _render_failure_groups(category: CategoryResult) -> List[str]:
    groups = _group_failures(category)
    if not groups:
        return []
    lines = ["## Failure Groups", ""]
    lines.append(
        "Failures collapsed by error signature. The count is the number of tests "
        "sharing that signature; the *first* test path is shown as an example."
    )
    lines.append("")
    lines.append("| # | Tests | Signature | Example |")
    lines.append("|--:|------:|-----------|---------|")
    for index, (signature, items) in enumerate(groups.items(), start=1):
        example = _truncate_path(items[0].path, category.category + "/")
        lines.append(
            "| %d | %d | %s | `%s` |"
            % (index, len(items), _md_escape(signature), _md_escape(example))
        )
    lines.append("")
    return lines


def _render_breakdown(category: CategoryResult) -> List[str]:
    """Per-subdirectory pass/fail breakdown."""

    buckets: "OrderedDict[str, Counter]" = OrderedDict()
    for item in category.results:
        relative = _truncate_path(item.path, category.category + "/")
        parts = relative.split("/")
        if len(parts) > 2:
            key = "/".join(parts[:2])
        elif len(parts) == 2:
            key = parts[0]
        else:
            key = "(top level)"
        buckets.setdefault(key, Counter())[item.status] += 1
    if not buckets:
        return []
    lines = ["## Breakdown by Subdirectory", ""]
    lines.append("| Subdirectory | Total | Pass | Fail | Skip | Not run |")
    lines.append("|--------------|------:|-----:|-----:|-----:|--------:|")
    for key in sorted(buckets):
        counts = buckets[key]
        total = sum(counts.values())
        lines.append(
            "| `%s` | %d | %d | %d | %d | %d |"
            % (
                _md_escape(key),
                total,
                counts.get(STATUS_PASSED, 0),
                counts.get(STATUS_FAILED, 0),
                counts.get(STATUS_SKIPPED, 0),
                counts.get(STATUS_NOT_RUN, 0) + counts.get(STATUS_UNKNOWN, 0),
            )
        )
    lines.append("")
    return lines


def _render_appendix(category: CategoryResult) -> List[str]:
    """Render the full per-test appendix table."""

    prefix = category.category + "/"
    lines = ["## Appendix — All Tests", ""]
    lines.append(
        "Every test file discovered in this category, with the path relative to "
        "`test262/test/%s/`, its status, and the error message or skip reason "
        "produced by the runner." % category.category
    )
    lines.append("")
    lines.append("| Test | Status | Message / Notes |")
    lines.append("|------|--------|-----------------|")
    for item in category.results:
        name = _truncate_path(item.path, prefix)
        name = _truncate_middle(name, MAX_TEST_NAME_CHARS)
        message = item.message or item.notes
        message = _truncate_middle(_md_escape(message), MAX_MESSAGE_CHARS)
        lines.append(
            "| `%s` | %s | %s |" % (_md_escape(name), STATUS_BADGE.get(item.status, item.status), message)
        )
    lines.append("")
    return lines


def _render_summary(category: CategoryResult) -> List[str]:
    counts = _summarize(category)
    total = len(category.results)
    executed = total - counts[STATUS_SKIPPED]
    pass_rate = (counts[STATUS_PASSED] / executed * 100.0) if executed else 0.0
    overall = (counts[STATUS_PASSED] / total * 100.0) if total else 0.0

    lines = ["## Summary", ""]
    lines.append("| Metric | Value |")
    lines.append("|--------|-------|")
    lines.append("| Category | `%s` |" % _md_escape(category.category))
    lines.append("| Test files discovered | %d |" % total)
    lines.append("| Chunks executed | %d |" % category.chunks)
    lines.append("| Parallel jobs | %d |" % category.jobs)
    lines.append("| Wall-clock duration | %s |" % _fmt_duration(category.duration))
    lines.append("| Pass rate (executed) | %.2f%% |" % pass_rate)
    lines.append("| Pass rate (all discovered) | %.2f%% |" % overall)
    lines.append("")
    lines.append("| Status | Count |")
    lines.append("|--------|------:|")
    for status in SUMMARY_ORDER:
        lines.append("| %s | %d |" % (STATUS_BADGE.get(status, status), counts[status]))
    lines.append("| **Total** | **%d** |" % total)
    lines.append("")
    return lines


def render_category_report(category: CategoryResult) -> str:
    """Render the Markdown report for a single category."""

    counts = _summarize(category)
    lines: List[str] = []
    lines.append("# Test262 %s — results" % category.category)
    lines.append("")
    lines.append("Generated by `scripts/test/test262-%s.py`." % category.category)
    lines.append("")
    lines.append("| | |")
    lines.append("|---|---|")
    lines.append("| Generated | %s |" % category.generated_at)
    lines.append("| Runner | `%s` |" % category.runner)
    lines.append("| Command | `%s` |" % _md_escape(category.command))
    lines.append("")
    lines.append(
        "**Result: %d passed, %d failed, %d skipped, %d not run of %d tests.**"
        % (
            counts[STATUS_PASSED],
            counts[STATUS_FAILED],
            counts[STATUS_SKIPPED],
            counts[STATUS_NOT_RUN] + counts[STATUS_UNKNOWN],
            len(category.results),
        )
    )
    lines.append("")

    lines.extend(_render_summary(category))

    if category.warnings:
        lines.append("## Runner Warnings")
        lines.append("")
        for warning in category.warnings:
            lines.append("- %s" % _md_escape(warning))
        lines.append("")

    lines.extend(_render_breakdown(category))
    lines.extend(_render_failure_groups(category))
    lines.extend(_render_appendix(category))

    return "\n".join(lines).rstrip() + "\n"


def write_category_report(category: CategoryResult, docs_root: Optional[Path] = None) -> Path:
    """Write ``docs/test/<category>/master.md`` and return its path."""

    root = Path(docs_root) if docs_root is not None else DOCS_ROOT
    target = root / category.category / "master.md"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(render_category_report(category), encoding="utf-8")
    return target


# ---------------------------------------------------------------------------
# Shared command-line driver
# ---------------------------------------------------------------------------


def runner_available() -> bool:
    return RUNNER_BIN.is_file() and os.access(RUNNER_BIN, os.X_OK)


def build_runner(*, quiet: bool = False) -> int:
    """Build the Rust example runner and return its exit code."""

    command = ["cargo", "build", "-r", "--example", "test262_runner"]
    if not quiet:
        _progress("building runner: " + " ".join(command))
    completed = subprocess.run(command, cwd=str(REPO_ROOT))
    return completed.returncode


def add_common_arguments(parser: argparse.ArgumentParser, category: str) -> None:
    parser.add_argument(
        "--jobs",
        type=int,
        default=DEFAULT_JOBS,
        help="number of chunk subprocesses to run in parallel (default: %(default)s)",
    )
    parser.add_argument(
        "--timeout-ms",
        type=int,
        default=CATEGORY_TIMEOUT_MS.get(category, DEFAULT_TEST_TIMEOUT_MS),
        help="per-test watchdog timeout in milliseconds (default: %(default)s)",
    )
    parser.add_argument(
        "--chunk-timeout",
        type=int,
        default=DEFAULT_CHUNK_TIMEOUT_S,
        help="wall-clock timeout for one chunk in seconds (default: %(default)s)",
    )
    parser.add_argument(
        "--only",
        action="append",
        default=None,
        metavar="SUBSTRING",
        help="only run chunks whose path contains SUBSTRING (repeatable)",
    )
    parser.add_argument(
        "--logs",
        type=Path,
        default=LOG_ROOT / category,
        help="directory for raw runner logs; kept outside the repository so the "
        "editor does not watch thousands of files (default: %(default)s)",
    )
    parser.add_argument(
        "--log-all",
        action="store_true",
        help="keep the raw log for passing chunks too (default: only failing "
        "chunks and chunks with warnings are saved)",
    )
    parser.add_argument(
        "--docs",
        type=Path,
        default=DOCS_ROOT,
        help="root directory for the generated Markdown (default: %(default)s)",
    )
    parser.add_argument(
        "--cache-dir",
        type=Path,
        default=CHUNK_CACHE_ROOT / category,
        help="directory for cached chunk results; a cached chunk is replayed "
        "instead of re-executed when nothing it depends on changed "
        "(default: %(default)s)",
    )
    parser.add_argument(
        "--no-cache",
        action="store_true",
        help="ignore and do not update the chunk result cache (always re-run "
        "every chunk)",
    )
    parser.add_argument(
        "--build",
        action="store_true",
        help="build the runner first if the binary is missing",
    )
    parser.add_argument(
        "--quiet",
        action="store_true",
        help="suppress per-chunk progress lines on stderr",
    )
    parser.add_argument(
        "--strict",
        action="store_true",
        help="exit with status 1 when at least one test failed",
    )


def run_category_cli(category: str, argv: Optional[Sequence[str]] = None) -> int:
    """Run one category, write its report, and return a process exit code."""

    parser = argparse.ArgumentParser(
        description="Run the Test262 %s category and write docs/test/%s/master.md" % (category, category)
    )
    add_common_arguments(parser, category)
    args = parser.parse_args(argv)

    if not runner_available():
        if args.build:
            code = build_runner(quiet=args.quiet)
            if code != 0:
                _progress("cargo build failed with exit code %d" % code)
                return code
        if not runner_available():
            _progress("runner binary not found: %s" % RUNNER_BIN)
            _progress("build it with: cargo build -r --example test262_runner")
            return 2

    result = run_category(
        category,
        jobs=args.jobs,
        timeout_ms=args.timeout_ms,
        chunk_timeout=args.chunk_timeout,
        log_dir=args.logs,
        only=args.only,
        quiet=args.quiet,
        log_all=args.log_all,
        cache_dir=None if args.no_cache else args.cache_dir,
    )

    report = write_category_report(result, args.docs)
    counts = _summarize(result)
    _progress(
        "%s: %d passed, %d failed, %d skipped, %d not run of %d -> %s"
        % (
            category,
            counts[STATUS_PASSED],
            counts[STATUS_FAILED],
            counts[STATUS_SKIPPED],
            counts[STATUS_NOT_RUN] + counts[STATUS_UNKNOWN],
            len(result.results),
            report,
        )
    )

    if args.strict and counts[STATUS_FAILED] > 0:
        return 1
    return 0


# ---------------------------------------------------------------------------
# Master (all-categories) report
# ---------------------------------------------------------------------------


def _render_master_appendix(categories: Sequence[CategoryResult]) -> List[str]:
    lines = ["## Appendix — All Tests", ""]
    lines.append(
        "Every test file discovered across all categories. Paths are relative to "
        "`test262/test/`; the category document has the same rows with the "
        "category prefix stripped."
    )
    lines.append("")
    for category in categories:
        counts = _summarize(category)
        lines.append("### %s" % category.category)
        lines.append("")
        lines.append(
            "%d passed, %d failed, %d skipped, %d not run of %d tests."
            % (
                counts[STATUS_PASSED],
                counts[STATUS_FAILED],
                counts[STATUS_SKIPPED],
                counts[STATUS_NOT_RUN] + counts[STATUS_UNKNOWN],
                len(category.results),
            )
        )
        lines.append("")
        if not category.results:
            lines.append("_No tests discovered._")
            lines.append("")
            continue
        lines.append("| Test | Status | Message / Notes |")
        lines.append("|------|--------|-----------------|")
        for item in category.results:
            name = _truncate_middle(item.path, MAX_TEST_NAME_CHARS + len(category.category) + 1)
            message = item.message or item.notes
            message = _truncate_middle(_md_escape(message), MAX_MESSAGE_CHARS)
            lines.append(
                "| `%s` | %s | %s |"
                % (_md_escape(name), STATUS_BADGE.get(item.status, item.status), message)
            )
        lines.append("")
    return lines


def _render_master_failure_groups(categories: Sequence[CategoryResult], limit: int = 60) -> List[str]:
    groups: "OrderedDict[str, List[TestResult]]" = OrderedDict()
    for category in categories:
        for item in category.results:
            if item.status != STATUS_FAILED:
                continue
            signature = _truncate_middle(item.message.strip() or "(no message)", 160)
            groups.setdefault(signature, []).append(item)
    if not groups:
        return []
    ordered = sorted(groups.items(), key=lambda kv: (-len(kv[1]), kv[0]))[:limit]
    lines = ["## Top Failure Groups (all categories)", ""]
    lines.append("| # | Tests | Signature | Example |")
    lines.append("|--:|------:|-----------|---------|")
    for index, (signature, items) in enumerate(ordered, start=1):
        lines.append(
            "| %d | %d | %s | `%s` |"
            % (index, len(items), _md_escape(signature), _md_escape(items[0].path))
        )
    lines.append("")
    return lines


def render_master_report(categories: Sequence[CategoryResult]) -> str:
    """Render the top-level ``docs/test/master.md`` report."""

    totals: "OrderedDict[str, int]" = OrderedDict((status, 0) for status in SUMMARY_ORDER)
    total_tests = 0
    total_duration = 0.0
    for category in categories:
        for status, count in _summarize(category).items():
            totals[status] += count
        total_tests += len(category.results)
        total_duration += category.duration

    executed = total_tests - totals[STATUS_SKIPPED]
    pass_rate = (totals[STATUS_PASSED] / executed * 100.0) if executed else 0.0
    overall = (totals[STATUS_PASSED] / total_tests * 100.0) if total_tests else 0.0

    generated = _datetime.datetime.now().astimezone().strftime("%Y-%m-%d %H:%M:%S %Z")
    runner = str(RUNNER_BIN)
    command = "python3 " + " ".join(sys.argv)

    lines: List[str] = []
    lines.append("# Test262 — master results")
    lines.append("")
    lines.append("Aggregated results for every Test262 category driven by `scripts/test/`.")
    lines.append("")
    lines.append("| | |")
    lines.append("|---|---|")
    lines.append("| Generated | %s |" % generated)
    lines.append("| Runner | `%s` |" % runner)
    lines.append("| Command | `%s` |" % _md_escape(command))
    lines.append("| Categories | %d |" % len(categories))
    lines.append("| Total wall-clock | %s |" % _fmt_duration(total_duration))
    lines.append("")
    lines.append(
        "**Result: %d passed, %d failed, %d skipped, %d not run of %d tests "
        "(pass rate %.2f%% of executed tests).**"
        % (
            totals[STATUS_PASSED],
            totals[STATUS_FAILED],
            totals[STATUS_SKIPPED],
            totals[STATUS_NOT_RUN] + totals[STATUS_UNKNOWN],
            total_tests,
            pass_rate,
        )
    )
    lines.append("")

    lines.append("## Overall Summary")
    lines.append("")
    lines.append("| Metric | Value |")
    lines.append("|--------|-------|")
    lines.append("| Test files discovered | %d |" % total_tests)
    lines.append("| Pass rate (executed) | %.2f%% |" % pass_rate)
    lines.append("| Pass rate (all discovered) | %.2f%% |" % overall)
    lines.append("")
    lines.append("| Status | Count |")
    lines.append("|--------|------:|")
    for status in SUMMARY_ORDER:
        lines.append("| %s | %d |" % (STATUS_BADGE.get(status, status), totals[status]))
    lines.append("| **Total** | **%d** |" % total_tests)
    lines.append("")

    lines.append("## Categories")
    lines.append("")
    lines.append(
        "| Category | Discovered | Pass | Fail | Skip | Not run | Pass rate | Duration | Report |"
    )
    lines.append(
        "|----------|-----------:|-----:|-----:|-----:|--------:|----------:|---------:|--------|"
    )
    for category in categories:
        counts = _summarize(category)
        total = len(category.results)
        executed_c = total - counts[STATUS_SKIPPED]
        rate = (counts[STATUS_PASSED] / executed_c * 100.0) if executed_c else 0.0
        lines.append(
            "| `%s` | %d | %d | %d | %d | %d | %.2f%% | %s | [details](%s/master.md) |"
            % (
                _md_escape(category.category),
                total,
                counts[STATUS_PASSED],
                counts[STATUS_FAILED],
                counts[STATUS_SKIPPED],
                counts[STATUS_NOT_RUN] + counts[STATUS_UNKNOWN],
                rate,
                _fmt_duration(category.duration),
                category.category,
            )
        )
    lines.append("")

    warnings = [(category.category, warning) for category in categories for warning in category.warnings]
    if warnings:
        lines.append("## Runner Warnings")
        lines.append("")
        for category_name, warning in warnings:
            lines.append("- `%s`: %s" % (_md_escape(category_name), _md_escape(warning)))
        lines.append("")

    lines.extend(_render_master_failure_groups(categories))
    lines.extend(_render_master_appendix(categories))

    return "\n".join(lines).rstrip() + "\n"


def write_master_report(categories: Sequence[CategoryResult], docs_root: Optional[Path] = None) -> Path:
    """Write ``docs/test/master.md`` and return its path."""

    root = Path(docs_root) if docs_root is not None else DOCS_ROOT
    target = root / "master.md"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(render_master_report(categories), encoding="utf-8")
    return target


# ===END===
