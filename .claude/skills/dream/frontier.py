#!/usr/bin/env python3
"""List unadjudicated pairs that touch nodes added since a given point.

Usage: python3 frontier.py <data_dir> <since_id> [candidate.json ...]

After an ingest, the pairs worth working are the ones involving the new nodes:
old-versus-old was already swept, so it is where the ledger is dense and the
remaining candidates score flat. This is a SCHEDULING rule, not a yield rule —
recency does not predict a finding (measured: 7.3 / 11.7 / 6.7 / 7.3 percent by
recency band, braim ID:1330). It just tells you which pairs are actually new.

<since_id> is the highest node id that existed before the ingest. Everything
above it is the frontier. `braim list` or the max id in current.json before the
ingest gives you the number.
"""
import json, sys, os, collections

if len(sys.argv) < 3:
    sys.exit(__doc__)

data_dir, since = sys.argv[1], int(sys.argv[2])
cand_files = sys.argv[3:]

G = json.load(open(os.path.join(data_dir, "current.json")))
N = G["nodes"]
seen = {frozenset((x["a"], x["b"]))
        for x in json.load(open(os.path.join(data_dir, "dreams.json")))}

DEP = {k: {str(x) for x in (v.get("depends_on") or {})} for k, v in N.items()}

def ancestors(k):
    out, stack = set(), [str(k)]
    while stack:
        for parent in DEP.get(stack.pop(), ()):
            if parent not in out:
                out.add(parent)
                stack.append(parent)
    return out

def adjudicable(a, b):
    for k in (a, b):
        if k not in N or N[k].get("invalid"):
            return False
        if N[k].get("verification_status") == "contested":
            return False
    return b not in ancestors(a) and a not in ancestors(b)

pairs = set()
for fn in cand_files:
    try:
        for p in json.load(open(fn)):
            pairs.add(frozenset((str(p["a"]), str(p["b"]))))
    except (OSError, ValueError, KeyError):
        continue

# Same-source-path pairs: braim's strategies do not emit these on their own,
# and they are where duplicates and contradictions concentrate.
MEASURABLE = ("code:", "schema:", "config:", "test:")
by_path = collections.defaultdict(list)
for k, v in N.items():
    if v.get("invalid"):
        continue
    for s in (v.get("sources") or []):
        if str(s).startswith(MEASURABLE):
            by_path[str(s)].append(k)
for ids in by_path.values():
    for i in range(len(ids)):
        for j in range(i + 1, len(ids)):
            pairs.add(frozenset((ids[i], ids[j])))

rows = []
for pr in pairs:
    if len(pr) != 2:
        continue
    a, b = tuple(pr)
    if frozenset((int(a), int(b))) in seen or not adjudicable(a, b):
        continue
    if max(int(a), int(b)) <= since:      # nothing new in this pair
        continue
    rows.append((min(int(a), int(b)) <= since, a, b))   # cross-era first

# A frontier node paired with an older one brings two different vintages of
# evidence together; frontier-versus-frontier is usually one ingest talking to
# itself. Order by that, not by score — the scores are flat by now.
rows.sort(key=lambda r: (not r[0], int(r[1])))
cross = sum(1 for r in rows if r[0])
print(f"unadjudicated pairs touching nodes above {since}: {len(rows)} "
      f"({cross} cross-era, {len(rows) - cross} frontier-only)\n")
for is_cross, a, b in rows[:40]:
    tag = "cross" if is_cross else "front"
    print(f"[{tag}] {a} {b}")
    print(f"    {N[a].get('label', '')[:96]}")
    print(f"    {N[b].get('label', '')[:96]}")
