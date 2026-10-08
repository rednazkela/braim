# Two Gaps in the Verification Model: Inferred Independence, and Uniform Re-Grounding

braim's whole trust model rests on PRIMARY-type diversity: two or three
independent source *types* agreeing is what promotes a claim from `partial`
to `proven` to `proven_strong`. Two things about that model don't hold as
cleanly as the type-counting math assumes, and both surfaced concretely
during a live dream session on the `profserv` graph rather than as abstract
worry.

## Gap 1: type diversity assumes independence it never checks

The promotion math counts *how many different PRIMARY types* cite a claim.
It has no way to check whether those citations made *independent contact*
with the thing they describe, versus one citing another.

**What this looks like in practice — ID:1499 vs ID:1511/1513.**

@[A third consolidation document exists, it does cover the movers and
Powercode, and no node cites it: tooling_etl_comparison.xlsx ... so the
omission ID:1345, ID:1347 and ID:1489 measure is a property of
piecemeal_tools.md and the Merge Tool Project Plan rather than of the
written record as a whole] source: braim ID:1499, doc:tooling_etl_comparison.xlsx

ID:1499 uses `tooling_etl_comparison.xlsx` as a `doc:` PRIMARY source to
conclude something new about the graph's own completeness. Opening the
actual file (not just trusting the label) shows why that's circular:

@[tooling_etl_comparison.xlsx cannot be used to verify the statements it
renders ... the spreadsheet subtitles itself Source braim graph v45] source:
braim ID:1511, doc:tooling_etl_comparison.xlsx

Directly confirmed by re-reading the file this session: sheet1's own header
row reads *"Source: braim graph v45 (transcripts + repo code sweeps)"* with
a dedicated `braim IDs` column citing prior graph node IDs per row. The
document is not independent evidence — it's the graph's own prior state,
exported and read back at itself. `doc:` and `code:`/`transcript:` on other
nodes are genuinely independent categories; `doc:` pointing at a
graph-derived artifact is not, and nothing in the type-counting math can
tell the difference. (Written up this session as ID:1656/ID:1660.)

The same failure mode is easy to construct outside this graph: a technical
document, a sales transcript, and a codebase can each carry a different
PRIMARY type and still be *one* observation wearing three costumes, if the
transcript's claim was read off the same stale doc the code was never
checked against. Type diversity is a proxy for independent contact with
reality, not a guarantee of it — and the gap is invisible from inside the
graph, because the only thing checking it is whichever LLM happens to be
dreaming that night, reading all three artifacts itself. The developer, the
documenter, and the sales rep each only ever see their own source; none of
them tags whether their claim was informed by one of the others.

**Proposed fix**: a `--derived-from` edge, distinct from `--depends`,
recorded at `source add` time by whoever is ingesting the source:

```
braim sources add "doc:tooling_etl_comparison.xlsx" --type doc \
  --derived-from "braim-graph-export"
```

Promotion logic would then be able to ask not just "how many PRIMARY types
cite this" but "how many of those types made independent contact with the
referent, versus contact with another source already in the set." A source
tagged `--derived-from braim-graph-export` (or, more generally, from another
cited source) shouldn't count toward type diversity at all — it's a
restatement, not a witness. This doesn't need to be inferred by the
promoting agent; it should be asserted by whoever attaches the source, the
same way `--sources` and `--domains` already are.

## Gap 2: RE-GROUND AT PROMOTION is a blanket rule, but citations aren't uniformly risky

The current policy — verify a claim's figures against its actual source
document before promoting, every time — is correct and caught a real problem
this session:

@[What the financial-recognition exclusion surface actually is remains
unproven ... code:app/Account.php:1930] source: braim ID:1548

Line 1930 of `Account.php` is `scopeEligibleForArchive` — it has nothing to
do with `frozen` at all. The citation was stale or mistyped, and the only
reason it surfaced was that the pre-pass forced an actual file read rather
than trusting the label. That's the policy working.

But the policy is applied identically to every citation regardless of how
likely it is to have drifted. A citation into a schema file that hasn't
changed in a year and a citation into a service class mid-active-development
carry very different risk of having rotted, and right now braim has no way
to tell them apart — every promotion pays the same re-grounding cost, and
every un-promoted claim carries the same residual risk of citing a moved
target, whether or not anything nearby has changed.

This is exactly the shape of a citation-staleness problem rctags is
positioned to answer (see the companion proposal,
`rctags-volatility-tracking-proposal.md`): a per-citation churn or
supersession score that RE-GROUND could use to prioritize *which* citations
get re-checked most urgently, rather than treating every citation as equally
suspect. Two smaller, related observations worth folding into the same
effort:

- **Weight-tuning can't see evidentiary concreteness, only type diversity.**
  During this session's weight-tuning pass, ID:1657's split was corrected
  because one dependency (a ClickUp board audit with named, individually
  checkable card IDs) demonstrated the claim more concretely than the other
  (a "never mentioned in 28 transcript files" corpus-silence claim) — despite
  both carrying comparable PRIMARY-type status. That distinction — how
  falsifiable a specific piece of evidence is, not just what type it is —
  currently requires an LLM to notice by rereading; there's no field for it.
- **`dream candidates --strategy shared-source` degrades badly on hub
  files.** A large, heavily-cited code directory or document (this session:
  `repos/powercode/powercode/src/Powercode`, `tooling_etl_comparison.xlsx`)
  generates candidate pairs purely because both nodes cite the same broad
  path, producing runs of false positives an adjudicator has to
  individually dismiss. Scoring shared-source candidates down when the
  shared path's total citation count is very high (a "hub" penalty,
  analogous to how a 5000-line file already gets down-weighted in the
  what-if staleness search) would likely raise shared-source's
  signal-to-noise ratio without losing real hits — real relations in this
  session came from files cited by a handful of nodes, never from the
  highest-citation hubs.

## Summary of proposed changes

1. Add `--derived-from` as a first-class edge type on sources, distinct from
   `--depends`, asserted at ingestion rather than inferred at promotion.
2. Exclude `--derived-from` sources from PRIMARY-type diversity counts for
   the node(s) they were derived from.
3. Surface a per-citation staleness/volatility score (from rctags or an
   equivalent) and use it to prioritize RE-GROUND effort rather than apply
   it uniformly.
4. Apply the same hub-file scoring penalty already used in what-if's
   staleness search to `dream candidates --strategy shared-source`.

None of these require touching the core `MIN(source-derived, weakest
dependency)` status math — they change what gets counted as independent
input to that math, and where re-grounding effort gets spent, not the
math itself.
