#!/usr/bin/env python3
"""Run the Test262 ``intl402/`` category and write ``docs/test/intl402/master.md``."""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from test262_common import run_category_cli  # noqa: E402

CATEGORY = "intl402"

if __name__ == "__main__":
    sys.exit(run_category_cli(CATEGORY))
