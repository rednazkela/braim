#!/usr/bin/env python3
"""Rank because_of edges whose causal claim is not backed by the node's evidence.

Usage: python3 suspect_edges.py [data_dir] [top_n]

A because_of edge is a causal claim and the only claim in this graph with no
source type of its own: all 340 edges cite a `narrative:` source, so the prefix
that grades a statement grades nothing here. The signal has to come from the
shape of the edge against the node it lands on.

THE DISCRIMINATOR, measured before it was used. When an author writes "this
happened because of that", the cause is usually also one of the reasons the
node exists — so the parent appears in the child's depends_on. That holds for
303 of 340 edges on this graph. The remaining 37 are edges whose causal claim
points one way while the node's evidence points entirely elsewhere: the author
asserted a cause and then built the statement out of something else. Those are
where an inverse test is most likely to fail.

WHAT WAS TRIED FIRST AND REJECTED. Ranking on id gap and shared creation hour
alone — the shape of writing a parent immediately before its child — flagged
282 of 340 edges, and its top three hits were genuine sequential reasoning
chains (a user reframe causing a user decision, and so on). A real cause very
often IS written just before its consequent, so proximity by itself marks the
normal shape of thinking, not a defect. Both signals are kept, but only as
corroboration on an edge already suspect for being outside depends_on.

UNTESTED IS THE GATE, not a signal. braim stores an inverse-test result on the
edge in `test_source`, and 317 of 340 carry none. An edge that already passed
`why-test` has been reasoned about and is withheld whatever its shape; one that
FAILED is already reported by `braim audit` under refuted causal links.

Nodes carrying metadata causal_regrounded_at are withheld, as whatif_walked_at,
reweighed_at and label_shape_at withhold their own.
"""
import json, sys, os

data_dir = sys.argv[1] if len(sys.argv) > 1 else ".braim"
top_n = int(sys.argv[2]) if len(sys.argv) > 2 else 10

path = os.path.join(data_dir, "current.json")
if not os.path.exists(path):
    sys.exit(f"no graph at {path}")
graph = json.load(open(path))
nodes = graph.get("nodes", {})
edges = [e for e in graph.get("because_of", []) if not e.get("invalid")]

GAP_NEAR = 3          # "the previous thing written", from the measured median of 2
SAME_HOUR = 13        # ISO prefix length through the hour: 2026-09-18T19

def created(nid):
    return (nodes.get(str(nid)) or {}).get("created_at") or ""

def label(nid):
    return ((nodes.get(str(nid)) or {}).get("label") or "")[:88]

scored = []
withheld_tested = 0
withheld_marked = 0
withheld_grounded = 0

for e in edges:
    child, parent = e.get("from"), e.get("to")
    if child is None or parent is None:
        continue
    if e.get("test_source"):
        withheld_tested += 1
        continue
    meta = (nodes.get(str(child)) or {}).get("metadata") or {}
    if meta.get("causal_regrounded_at"):
        withheld_marked += 1
        continue

    gap = child - parent
    c_at, p_at = created(child), created(parent)
    same_hour = bool(c_at and p_at and c_at[:SAME_HOUR] == p_at[:SAME_HOUR])
    child_deps = set(int(k) for k in ((nodes.get(str(child)) or {}).get("depends_on") or {}))
    parent_carries_weight = parent in child_deps

    # The gate: a causal claim corroborated by the node's own dependency set is
    # not what this pass is looking for, however close the two were written.
    if parent_carries_weight:
        withheld_grounded += 1
        continue

    score = 0.6
    why = ["parent carries no dependency weight"]
    if 0 < gap <= GAP_NEAR:
        score += 0.25
        why.append(f"parent {gap} id(s) back")
    if same_hour:
        score += 0.15
        why.append("same creation hour")
    scored.append((score, child, parent, "; ".join(why)))

scored.sort(key=lambda r: (-r[0], r[1]))

print(f"because_of edges scanned: {len(edges)}")
print(f"withheld, already inverse-tested: {withheld_tested}")
print(f"withheld, already re-grounded: {withheld_marked}")
print(f"withheld, parent also a dependency: {withheld_grounded}")
print(f"suspect edges: {len(scored)}\n")

for score, child, parent, why in scored[:top_n]:
    print(f"{score:.2f}  ID:{child} --because_of--> ID:{parent}   [{why}]")
    print(f"      child : {label(child)}")
    print(f"      parent: {label(parent)}")
    print()

if scored:
    print("Test one with: braim why-test <child_id>            # cause confirmed")
    print("               braim why-test <child_id> --fail     # consequent stands without it")
    print("Then:          braim why-remove <child_id> && braim why-add <child_id> --because <real_parent>")
    print("               or braim meta <child_id> --set terminal_cause=true")
    print("Mark either way: braim meta <child_id> --set causal_regrounded_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)")
