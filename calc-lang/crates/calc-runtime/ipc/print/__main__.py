"""The `print` built-in's IPC implementation (spec.md §7 kind 3; sessions A11, A12).

A small multi-file project, run as `python <this directory>`: Python runs a
directory's `__main__.py` with that directory first on `sys.path`, so sibling
modules import normally. It speaks calc-lang's IPC protocol: one JSON request
(`{"args": [...]}`) on stdin, one JSON response (`{"output": ..., "result": ...}`)
on stdout. See calc-lang/docs/a12-the-link-driver.md.
"""

import json
import sys

from formatting import decode, describe, encode

request = json.load(sys.stdin)
x = decode(request["args"][0])
json.dump({"output": describe(x), "result": encode(x)}, sys.stdout)
