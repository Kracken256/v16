#!/usr/bin/env python3
"""Run the Test262 ``language/`` category and write ``docs/test/language/master.md``.

The heavy lifting lives in :mod:`test262_common`; this script only pins the
category name.  See ``python3 scripts/test/test262-language.py --help`` for the
available options (``--jobs``, ``--only``, ``--strict``, ...).
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from test262_common import run_category_cli  # noqa: E402

CATEGORY = "language"

if __name__ == "__main__":
    sys.exit(run_category_cli(CATEGORY))
