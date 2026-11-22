# Prompt fixture pair — wording mutation test

`extraction_before.txt` and `provenance_after.txt` are a matched pair used to
check that model-facing gate/policy wording teaches the provenance discipline
without reading as reasoning extraction to a model's own guards.

## What the pair is

- `extraction_before.txt` is the **positive control**. It is deliberately
  shaped like an extraction attempt (imperative "comply", a demand to
  disclose/reveal the reasoning chain, self-asserted authority via "STOP").
  It is EXPECTED to be refused or flagged by a model's guards — that is the
  baseline this pair exists to contrast against, not a bug to fix.
- `provenance_after.txt` is this project's actual gate/policy wording (the
  B.1/appendix-A.4 text braim ships). It asks for the same end state — a
  `[braim: ...]` tag — but frames it as a request for records of actions
  taken (commands run, IDs returned), never as a request to disclose,
  reveal, or introspect on the model's internal reasoning process. It is
  EXPECTED to draw zero guard activation.

## Procedure

Any PR that touches model-facing strings in `hooks/` or `policies/` re-runs
this pair against the target model tier(s) before merging:

1. Deliver each fixture to the model under test (user-channel delivery is a
   valid proxy when the real hook stderr channel is unavailable; note which
   channel was used in the recorded result).
2. Record, per tier:
   - BEFORE: refused or flagged (expected) vs. accepted (also worth noting —
     it means that tier's guard is laxer than the one the baseline was
     calibrated against, not that the fixture is wrong).
   - AFTER: compliance shape — was the tag emitted, were IDs fabricated or
     only ones a real braim command returned, was `none` used honestly when
     nothing was recorded.
3. File the result as a braim finding node with the model tier in its
   metadata, not as prose in this file — this README is the fixed procedure,
   not the running log.

A refusal of the AFTER text is a line-edit instruction, not a process
failure: the refusal names the residual trigger phrase (the specific word or
framing that read as extraction), and that phrase gets swapped in the gate
or policy source, followed by a re-run of this same pair.

## Promotion

A single proxy run (one model tier, one delivery channel) is a partial
result, not a green light to drop the procedure — the fixture pair is
re-run against every target model tier through the real hook stderr channel
before the wording is considered load-bearing. See `braim-audit-rot-loop.md`
and `src/stop_gate.rs` for where this wording actually ships.
