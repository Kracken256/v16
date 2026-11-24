#!/usr/bin/env python3
"""Run every Test262 category and write ``docs/test/master.md``.

This is the wrapper that drives the per-category scripts.  It runs each
category in turn (each category itself runs its chunks in parallel), writes
``docs/test/<category>/master.md`` for every category, and finally writes the
aggregated ``docs/test/master.md``.

Examples::

    python3 scripts/test/test262-all.py                 # everything
    python3 scripts/test/test262-all.py --jobs 16       # more parallelism
    python3 scripts/test/test262-all.py --categories language,built-ins
    python3 scripts/test/test262-all.py --build --strict
"""

from __future__ import annotations

import argparse
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from test262_common import (  # noqa: E402
    CATEGORIES,
    CATEGORY_TIMEOUT_MS,
    CHUNK_CACHE_ROOT,
    DEFAULT_CHUNK_TIMEOUT_S,
    DEFAULT_JOBS,
    DEFAULT_TEST_TIMEOUT_MS,
    DOCS_ROOT,
    LOG_ROOT,
    STATUS_FAILED,
    STATUS_NOT_RUN,
    STATUS_PASSED,
    STATUS_SKIPPED,
    STATUS_UNKNOWN,
    _fmt_duration,
    _progress,
    _summarize,
    build_runner,
    run_category,
    runner_available,
    write_category_report,
    write_master_report,
)


def _parse_categories(raw: str) -> list:
    names = [item.strip() for item in raw.split(",") if item.strip()]
    unknown = [name for name in names if name not in CATEGORIES]
    if unknown:
        raise SystemExit(
            "unknown category/categories: %s (known: %s)" % (", ".join(unknown), ", ".join(CATEGORIES))
        )
    return names


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(
        description="Run every Test262 category and write docs/test/master.md",
    )
    parser.add_argument(
        "--categories",
        default=",".join(CATEGORIES),
        help="comma-separated categories to run (default: %(default)s)",
    )
    parser.add_argument(
        "--jobs",
        type=int,
        default=DEFAULT_JOBS,
        help="parallel chunk subprocesses per category (default: %(default)s)",
    )
    parser.add_argument(
        "--timeout-ms",
        type=int,
        default=None,
        help="per-test watchdog timeout in milliseconds; overrides the "
        "per-category default (default: %s for most categories, %s for harness)"
        % (DEFAULT_TEST_TIMEOUT_MS, CATEGORY_TIMEOUT_MS.get("harness")),
    )
    parser.add_argument(
        "--chunk-timeout",
        type=int,
        default=DEFAULT_CHUNK_TIMEOUT_S,
        help="wall-clock timeout for one chunk in seconds (default: %(default)s)",
    )
    parser.add_argument(
        "--logs",
        type=Path,
        default=LOG_ROOT,
        help="root directory for raw runner logs; kept outside the repository so "
        "the editor does not watch thousands of files (default: %(default)s)",
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
        default=CHUNK_CACHE_ROOT,
        help="root directory for cached chunk results; a chunk that completed "
        "without failures is replayed instead of re-executed when nothing it "
        "depends on changed (default: %(default)s)",
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
    args = parser.parse_args(argv)

    selected = _parse_categories(args.categories)

    if not runner_available():
        if args.build:
            code = build_runner(quiet=args.quiet)
            if code != 0:
                _progress("cargo build failed with exit code %d" % code)
                return code
        if not runner_available():
            _progress("runner binary not found; build it with:")
            _progress("  cargo build -r --example test262_runner")
            return 2

    started = time.monotonic()
    results = []
    interrupted = False
    try:
        for index, category in enumerate(selected, start=1):
            _progress("")
            _progress("=== [%d/%d] %s ===" % (index, len(selected), category))
            category_result = run_category(
                category,
                jobs=args.jobs,
                timeout_ms=args.timeout_ms,
                chunk_timeout=args.chunk_timeout,
                log_dir=args.logs / category,
                quiet=args.quiet,
                log_all=args.log_all,
                cache_dir=None if args.no_cache else args.cache_dir / category,
            )
            report = write_category_report(category_result, args.docs)
            counts = _summarize(category_result)
            _progress(
                "  %s: %d passed, %d failed, %d skipped, %d not run of %d -> %s"
                % (
                    category,
                    counts[STATUS_PASSED],
                    counts[STATUS_FAILED],
                    counts[STATUS_SKIPPED],
                    counts[STATUS_NOT_RUN] + counts[STATUS_UNKNOWN],
                    len(category_result.results),
                    report,
                )
            )
            results.append(category_result)
    except KeyboardInterrupt:
        # A interrupted run must still leave a usable report behind.  The
        # unfinished categories are simply absent from the master document, and
        # the cache lets the next invocation pick up where this one stopped.
        interrupted = True
        _progress("")
        _progress("interrupted; writing partial report for %d category/ies" % len(results))

    master = write_master_report(results, args.docs)
    elapsed = time.monotonic() - started

    totals = {status: 0 for status in (STATUS_PASSED, STATUS_FAILED, STATUS_SKIPPED, STATUS_NOT_RUN, STATUS_UNKNOWN)}
    total_tests = 0
    for category_result in results:
        for status, count in _summarize(category_result).items():
            totals[status] += count
        total_tests += len(category_result.results)

    _progress("")
    _progress("=== TOTAL ===")
    _progress(
        "%d passed, %d failed, %d skipped, %d not run of %d tests in %s"
        % (
            totals[STATUS_PASSED],
            totals[STATUS_FAILED],
            totals[STATUS_SKIPPED],
            totals[STATUS_NOT_RUN] + totals[STATUS_UNKNOWN],
            total_tests,
            _fmt_duration(elapsed),
        )
    )
    _progress("master report: %s" % master)

    if interrupted:
        _progress("run was interrupted; re-run to resume from the cached chunks")
        return 130
    if args.strict and totals[STATUS_FAILED] > 0:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
