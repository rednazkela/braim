#!/usr/bin/env python3
"""Print every live node citing any of the given source paths. Run BEFORE writing.

Usage: python3 whocites.py <data_dir> <path-fragment> [<path-fragment> ...]

The lookup-first rule fails when the query is phrased in the new finding's own
words: two statements about the same file need not share a single content word,
but they must share the file path. Five duplicates in one session came from
querying prose (braim ID:1233, ID:1284). This queries the path instead.

Pass the exact fragments you are about to put in --sources. Anything printed is
a node that already speaks about that file — read it before adding a statement.
"""
import json, sys, os

if len(sys.argv) < 3:
    sys.exit(__doc__)

data_dir, fragments = sys.argv[1], sys.argv[2:]
N = json.load(open(os.path.join(data_dir, "current.json")))["nodes"]

# Normalise a source string to the comparable part of its path.
#
# Two things defeat naive substring matching, both measured:
#  - a trailing :line-range, so "foo.py:13-29" must still match "foo.py:21-23";
#  - a differing repo-root prefix, so "code:sonar/app/Nova/Actions" must still
#    match "code:app/Nova/Actions". That one let duplicate number six through
#    (braim ID:1177 vs ID:1296) even though this script was run first.
def stems(frag):
    parts = frag.split(":")
    path = parts[1] if len(parts) > 1 else parts[0]
    if len(parts) > 2 and any(c.isdigit() for c in parts[-1]):
        path = ":".join(parts[1:-1])
    return [s for s in path.split("/") if s]

wanted = [stems(f) for f in fragments]

def related(a, b):
    """True when two segment lists name the same file or nest one inside the other.

    Compared from the tail so a differing repo-root prefix does not matter
    ("sonar/app/Nova/Actions" vs "app/Nova/Actions"), and a shorter path that is
    a parent directory of the longer one counts ("app/Nova" vs
    "app/Nova/Actions"). Requires at least two shared trailing segments, which
    keeps "app" or "src" from matching everything — the mistake that made the
    first attempt at this return the whole graph.
    """
    shared = 0
    for x, y in zip(reversed(a), reversed(b)):
        if x != y:
            break
        shared += 1
    if shared >= min(len(a), len(b)):      # one nests inside the other
        return min(len(a), len(b)) >= 2 or a == b
    return shared >= 2

hits = 0
for k, v in sorted(N.items(), key=lambda kv: int(kv[0])):
    if v.get("invalid"):
        continue
    srcs = [str(s) for s in (v.get("sources") or [])]
    matched = [s for s in srcs if any(related(stems(s), w) for w in wanted)]
    if not matched:
        continue
    hits += 1
    print(f"ID:{k}  [{v.get('node_type')}/{v.get('verification_status')}]")
    print(f"    {v.get('label', '')[:300]}")
    print(f"    cites: {matched}")

if not hits:
    print("no live node cites those paths — safe to add")
else:
    print(f"\n{hits} node(s) already cite these paths. Read them before writing.")
