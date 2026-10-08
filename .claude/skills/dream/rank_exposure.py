#!/usr/bin/env python3
"""Rank unverified nodes by how much unadjudicated pair surface rests on them.

Usage: python3 rank_exposure.py [data_dir] [top_n]

A node scores if it is weak (unproven or contested) AND carries a measurable
source (code/schema/config/test) — i.e. a claim you could settle by running a
command. The score is the number of candidate pairs not yet in dreams.json that
the node appears in. Measuring the top few puts that many future adjudications
on checked ground instead of on an unverified premise.

Reads the candidate files the caller has already dumped to <data_dir>/../ or a
scratch path passed as argv[3..]; falls back to scanning every node pair that
shares a source path, which is the surface braim's own strategies never emit.
"""
import json, sys, collections, os

data_dir = sys.argv[1] if len(sys.argv) > 1 else ".braim"
top_n = int(sys.argv[2]) if len(sys.argv) > 2 else 10
cand_files = sys.argv[3:]

G = json.load(open(os.path.join(data_dir, "current.json")))
N = G["nodes"]
dreams = json.load(open(os.path.join(data_dir, "dreams.json")))
seen = {frozenset((x["a"], x["b"])) for x in dreams}

MEASURABLE = ("code:", "schema:", "config:", "test:")
WEAK = ("unproven", "contested")

def sources(k):
    return [str(s) for s in (N[str(k)].get("sources") or [])]

def measurable(k):
    s = sources(k)
    return bool(s) and any(x.startswith(MEASURABLE) for x in s)

def weak(k):
    return N[str(k)].get("verification_status") in WEAK

PRIMARY = ("code:", "doc:", "schema:", "config:", "transcript:", "test:")

def already_fully_sourced(k):
    """Two distinct PRIMARY types already => source-derived status is proven.

    Such a node displays as unproven only because a dependency caps it, and
    verification never propagates (braim ID:1251), so no measurement can move
    it. Ranking it wastes the pre-pass on nodes that need their parents
    verified, not their own labels checked.
    """
    types = {s.split(":")[0] for s in sources(k) if s.startswith(PRIMARY)}
    return len(types) >= 2

def target(k):
    # Any metadata.measured value means a previous pre-pass already ran the
    # command: "unfixable" (cited file absent from this checkout), "refuted"
    # (measurement disagreed, correction recorded), and so on. Confirmed nodes
    # leave the ranking on their own by being promoted. Without this check a
    # node the pre-pass cannot move ranks first forever — ID:684 topped two
    # consecutive rounds before the marker was added.
    if (N[str(k)].get("metadata") or {}).get("measured"):
        return False
    if already_fully_sourced(k):
        return False
    return str(k) in N and weak(k) and measurable(k)

# Adjacency for the ancestor test. A pair whose sides are already on one
# dependency chain is not an open question, and counting those inflated the
# first version of this ranking by 40x on the node it was tested against.
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
    """Would this pair actually reach a verdict, or be filtered out first?"""
    for k in (a, b):
        if N[k].get("invalid") or N[k].get("verification_status") == "contested":
            return False
    return b not in ancestors(a) and a not in ancestors(b)

pairs = set()
for fn in cand_files:
    try:
        for p in json.load(open(fn)):
            pairs.add(frozenset((str(p["a"]), str(p["b"]))))
    except (OSError, ValueError, KeyError):
        continue

# Same-source-path pairs: braim's strategies do not emit these unless another
# signal coincides, yet they are where duplicates and contradictions concentrate.
by_path = collections.defaultdict(list)
for k, v in N.items():
    if v.get("invalid"):
        continue
    for s in sources(k):
        if s.startswith(MEASURABLE):
            by_path[s].append(k)
for ids in by_path.values():
    for i in range(len(ids)):
        for j in range(i + 1, len(ids)):
            pairs.add(frozenset((ids[i], ids[j])))

exposure = collections.Counter()
for pr in pairs:
    if len(pr) != 2:
        continue
    a, b = tuple(pr)
    if frozenset((int(a), int(b))) in seen:
        continue
    if a not in N or b not in N:
        continue
    if not adjudicable(a, b):
        continue
    for k in (a, b):
        if target(k):
            exposure[k] += 1

print(f"unadjudicated pairs scanned: {len(pairs)}")
print(f"weak measurable nodes with exposure: {len(exposure)}\n")
for k, n in exposure.most_common(top_n):
    v = N[k]
    print(f"ID:{k}  exposure {n}  [{v.get('verification_status')}]  "
          f"types={sorted({s.split(':')[0] for s in sources(k)})}")
    print(f"    {v['label'][:110]}")
