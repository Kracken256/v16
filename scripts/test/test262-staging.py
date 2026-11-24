#!/usr/bin/env python3
"""Run the Test262 ``staging/`` category and write ``docs/test/staging/master.md``.

``staging/`` holds proposal tests that have not been folded into the main
specification yet.  Most of them are skipped by the runner because of their
``features`` entries, but the report still lists every discovered file.
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from test262_common import run_category_cli  # noqa: E402

CATEGORY = "staging"

if __name__ == "__main__":
    sys.exit(run_category_cli(CATEGORY))
