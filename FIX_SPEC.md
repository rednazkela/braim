# BRAIM — Maintainer Fix Spec

Five issues surfaced by an LLM behavior test suite. Each is reproducible, each
has a one-liner symptom, and each is accompanied by a runnable test case in
`test_fixes.sh` (sibling file).

The test suite ran 17 scenarios against `braim` (current build), one per LLM
sub-agent. Source code paths cited below refer to the version of `braim` that
shipped `braim --help` output containing the "REQUIRED RULES" section and the
"SOURCE TYPES" taxonomy (PRIMARY / SECONDARY / TERTIARY).

---

## ISSUE 1 — duplicate-string entries in `--sources` accepted silently

**Symptom**

```
$ braim statement add "X" --domains a,b,c \
    --sources "doc:c.txt,doc:c.txt,doc:c.txt" \
    --depends "1:0.3,2:0.4,3:0.3"
✓ statement added
```

The same source string is repeated three times to satisfy the arity rule
(`--domains == --sources == --depends`). braim accepts the input even though
the three "sources" carry the same information.

**Observed in production**

612 nodes in audited graphs carry repeated identical source strings; ~37% of
recent task nodes pad this way. Test scenario S04 and S15 both reproduce.

**Why it matters**

Source padding silently bypasses the verification taxonomy. A statement with
three identical `doc:` strings reads as "three sources" in casual inspection
but provides no more evidence than a single `doc:` source. Distinct citations
(e.g. `doc:c.txt:21`, `doc:c.txt:45`) would carry real information.

**Proposed fix**

In the source-parser, after splitting on commas, detect duplicate entries
(case-sensitive string equality) and:

- by default: emit a warning to stderr  
  `⚠ duplicate source entries detected: ["doc:c.txt"×3]. Consider using
   distinct citations (line numbers, sections) per source slot.`
- with `--strict-sources` flag (new): reject with non-zero exit and the
  message above as an error.

The statement should still be written by default (warn-mode), since legacy
tooling may rely on the current permissiveness.

**Acceptance**

`test_fixes.sh` cases `t1_dup_sources_warn` and `t1_dup_sources_strict`.

---

## ISSUE 2 — PRIMARY + TERTIARY source types mix freely on the same node

**Symptom**

```
$ braim statement add "Inferred conclusion" --domains x,y \
    --sources "doc:c.txt:21-26,inference:arithmetic-derivation" \
    --depends "1:0.6,2:0.4"
✓ statement added
```

The statement carries both a PRIMARY source (`doc:`) and a TERTIARY source
(`inference:`). Per `braim --help`'s source taxonomy, TERTIARY sources do not
contribute to verification — they are derivations layered on top of evidence.
Mixing the two types muddies what "partial" verification means and produces
a node whose audit reading is ambiguous.

**Observed in production**

12 nodes in audited graphs mix `inference:`/`logic:` with `code:`/`doc:` on the
same statement. Test scenario S17 reproduces deterministically: when asked to
record an inferred observation citing both passages and reasoning, the LLM
puts both into `--sources`.

**Why it matters**

The verification status semantics (`unproven`/`partial`/`proven`/`proven_strong`)
depend on counting distinct PRIMARY source types. A mixed node currently
shows `partial` (one PRIMARY type), but a reader can't tell whether the
TERTIARY entry "counts" toward anything. Cleaner: record evidence in PRIMARY
sources, record reasoning in the label or in a separate statement that
`--depends` on the evidence statement.

**Proposed fix**

In source-parser, classify each entry by prefix into PRIMARY / SECONDARY /
TERTIARY. If a single `--sources` list contains both PRIMARY and TERTIARY
entries:

- by default: warn  
  `⚠ source taxonomy mix: PRIMARY (doc:) and TERTIARY (inference:) on the
   same statement. Inference is a derivation, not evidence — prefer
   PRIMARY-only sources here, and record reasoning in label or as a
   separate inference-only statement that --depends on this one.`
- with `--strict-sources`: reject.

SECONDARY (`phase_N:`, `agent:`, `narrative:`) mixed with PRIMARY remains
acceptable (no warning), since SECONDARY are contextual not derivational.

**Acceptance**

`test_fixes.sh` case `t2_primary_tertiary_mix_warn`.

---

## ISSUE 3 — duplicate `--domains` entries accepted silently

**Symptom**

```
$ braim statement add "X" --domains library,library,library \
    --sources "doc:a:1,doc:a:2,doc:a:3" \
    --depends "1:0.3,2:0.3,3:0.4"
✓ statement added
  Domains: ["library", "library", "library"]
```

The same domain string is repeated to satisfy the `--domains == --sources ==
--depends` arity rule. The resulting domain array is meaningless for query
purposes but counts toward arity equality.

**Observed in production**

612 nodes across audited graphs carry repeated domain entries. The most
extreme observed instance: a single node with `["computation"] × 10`.

**Why it matters**

`braim list --domain X` and `braim query` rely on domains for topical filtering.
Padded duplicates inflate domain occurrence counts (a node "tagged" with
`library` ten times looks more relevant than a node tagged once) and obscure
which domains a node truly belongs to.

**Proposed fix**

In the domain-parser, detect duplicates:

- by default: warn  
  `⚠ duplicate domain entries detected: ["library"×3]. The arity rule
   requires count equality, not value equality — consider using distinct
   domains (e.g. "library,operations,finance") per dependency slot.`
- with `--strict-domains` flag (new): reject.

Optional secondary improvement: store the domain array de-duplicated at
write time and adjust arity-check semantics so duplicates do not contribute
to the count (this is a larger change and out of scope for the minimum fix).

**Acceptance**

`test_fixes.sh` cases `t3_dup_domains_warn` and `t3_dup_domains_strict`.

---

## ISSUE 4 — gap register does not clear when a connecting structure is added

**Symptom**

```
$ braim audit
── Gap register ──
  ✗ ID:2 ↔ ID:4   No path found

$ braim statement add "2 connects to 4 because Z" \
    --depends "2:0.5,4:0.5" --domains x,y --sources "doc:z,doc:z"
✓ statement added (ID:99)

$ braim audit
── Gap register ──
  ✗ ID:2 ↔ ID:4   No path found  # still flagged
```

After an LLM (or human) investigates a registered gap and adds a connecting
statement, the gap remains in the register. Audit re-prompts forever even
though the pair has been addressed.

**Observed in production**

15 unresolved gap-register entries in audited graphs. Test scenario S16
reproduces — the LLM correctly investigates each gap, adds connecting
statements, then notes that braim still flags the original entries.

**Why it matters**

The gap register is supposed to be a worklist. If it never clears, it stops
being a worklist and becomes noise. Operators learn to ignore it; real gaps
get lost in the persistent list.

**Proposed fix**

Two paths, pick one:

(a) **Auto-clear on connect.** After every `braim statement add`, for each
    pair of `--depends` IDs (A, B), check if `(A,B)` or `(B,A)` is in
    `data.gaps`. If yes, remove it.

(b) **Manual clear with audit trail.** Add `braim audit clear-gap <A> <B>
    --reason "..."` that removes the entry and writes a small audit
    statement explaining the closure. Default audit-list shows only OPEN
    gaps; `--include-closed` shows the audit trail.

(a) is simpler. (b) is more defensible for graphs where "no connection
warranted" is a real outcome (S16 records "Reading Room and Library Card
are deliberately independent"); (b) lets that decision be auditable.

If picking (a), still document in --help that the clear is heuristic — two
concepts sharing a statement dependency does not always mean the
semantic gap is truly resolved.

**Acceptance**

`test_fixes.sh` case `t4_gap_register_clears_on_connect`.

---

## ISSUE 5 — atomic concept added whose label decomposes into existing atomics

**Symptom**

```
$ braim concept add "Library" --domains x --sources "doc:c:1"
✓ atomic concept added (ID:10)

$ braim concept add "Card" --domains x --sources "doc:c:2"
✓ atomic concept added (ID:11)

$ braim concept add "Library Card" --domains x --sources "doc:c:3"
✓ atomic concept added (ID:12)
```

The new atomic "Library Card" decomposes into two existing atomics ("Library"
and "Card") but braim does not flag the opportunity to create a compound.

**Observed in production**

This is the lone failure-cluster root cause in our test suite (S02 → S03
cascade): an LLM creates 17 atomics in S01, several of which are multi-word
names whose parts could be atomics themselves. S02 then can't proactively
re-cast them as compounds because the parts don't exist in the graph.

**Why it matters**

The compound-from-atomics structure is a load-bearing feature of braim's
semantic graph (`perspective`, `proximity`, weight propagation all rely on
it). When multi-word concepts are added as flat atomics, the graph loses
the structure that makes traversal queries meaningful.

**Proposed fix**

Heuristic-warn at concept-add time:

```
def warn_decomposable(label):
    tokens = label.split()
    if len(tokens) < 2:
        return
    matches = [t for t in tokens
               if find_atomic_by_exact_label(t) is not None]
    if len(matches) >= 2:
        warn(f"label '{label}' contains existing atomic names: "
             f"{matches}. Consider adding this as a compound depending on "
             f"those atomics: braim concept add '{label}' --depends "
             f"'{','.join(...)}'")
```

Heuristic, opt-out via `--no-decompose-hint`. Stays a warning; never blocks
the write.

A complementary heuristic: at `braim concept add` of an atomic whose label
ends with an existing-atomic suffix (e.g. "Late Fee" while "Fee" exists),
warn about the same.

**Acceptance**

`test_fixes.sh` case `t5_multiword_atomic_warns`.

---

## PRIORITIZATION

| Issue | Severity | Effort | Recommended order |
|-------|----------|--------|-------------------|
| 1 (dup sources) | high — silent verification corruption | small (~30 LOC) | first |
| 2 (PRIMARY+TERTIARY mix) | high — taxonomy contradiction | small (~40 LOC) | second |
| 3 (dup domains) | medium — query relevance noise | small (~30 LOC) | third |
| 4 (gap register stale) | medium — operator confusion | medium (~80 LOC, option a) | fourth |
| 5 (multi-word atomic hint) | low — graph quality only | small (~25 LOC) | fifth |

Issues 1-3 share a parser-level concern (taxonomy hygiene) and could land in
one PR. Issues 4 and 5 are independent.

## COMPATIBILITY

All five fixes default to **warn** behavior (write proceeds). Existing
graphs continue to load unchanged. Adding `--strict-*` flags is the
escape hatch for projects that want hard enforcement.

## OUT OF SCOPE

- Bulk re-write of existing graphs to remove documented padding. Operators
  can run a small migration script; that's not a tool-side concern.
- Single-use-domain proliferation warnings (~144 single-use domains in
  audited graphs). Worth considering as Issue 6 later, but ambiguous —
  a single-use domain may be perfectly correct.
- `inference:legacy_unclassified`-only nodes (530 occurrences). Pure
  migration debt from older importers; not an LLM-behavior issue.

## EVIDENCE PACKAGE

The test suite that produced these findings, including the 17 reproducible
scenario prompts and the corpus they operate on, lives at
`~/braim-tests/`. The recorded run including final graph state and per-
scenario verdict is at `~/braim-test-run/`. See
`~/braim-test-run/scoreboard.txt` for the per-scenario tally.
