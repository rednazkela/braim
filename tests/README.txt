BRAIM TEST PROJECT

A small validation suite for the `braim` semantic graph tool. Each scenario asks an
LLM to map portions of a short source document into braim nodes, statements, and
relationships, then a separate operator-side oracle measures whether the resulting
graph exhibits the properties braim is designed to produce.

The source document (corpus.txt) is an ordinary article about a small public
library. It contains the kinds of language braim is built to capture:
single-word concepts, multi-word concepts where adjacent nouns form a unit,
hierarchical decompositions, statements with multiple cited sources, and at
least one contradiction between an older and a newer claim.

FOLDER STRUCTURE

  README.txt              this file
  corpus.txt              the source document (one page of prose)
  scenario_01.txt..26.txt twenty-six scenario prompts handed to the test LLM
                          (01-08 cover basic feature coverage; 09-14 cover
                          usage violations observed in real-world graphs;
                          15-20 cover cross-source verification primitives;
                          21-23 cover the agent-integration policies;
                          24-26 cover the memory evidence-discipline traits,
                          26 scored on the saved reply text)
  corpus_appendix.txt     second source document (digital services), the
                          ingestion target for scenarios 24-26
  ../policies/            (repo root) the agent-integration policies and traits validated
                          by 21-26 (perturn_logging.json, compaction_rule.txt,
                          memory_braim_traits.md, README.md)
  oracle.txt              operator-side audit criteria (NOT shown to LLM)
  run.txt                 suggested run procedure

HOW TO USE

  1. Initialize a fresh braim graph in a new directory:
       mkdir braim-run && cd braim-run
       cp <repo>/tests/corpus.txt <repo>/tests/corpus_appendix.txt .
       cp -r <repo>/policies .
       braim --help > /dev/null
       (braim auto-creates ./.braim/current.json on first command)

  2. From a parent Claude Code session, invoke ONE sub-agent per scenario via
     the Agent tool, in order from scenario_01 through scenario_26. Each
     agent's prompt is the verbatim contents of the corresponding
     scenario_NN.txt file plus the working directory path. Do not include
     oracle.txt, the other scenarios, or any framing about what feature is
     under test.

     Every scenario file begins with a "BEFORE YOU BEGIN" block instructing
     the agent to run `braim --help` first. This is mandatory: each fresh
     agent must read the help text directly rather than relying on prior
     assumptions about how the tool works.

  3. Run agents sequentially against the same working directory; scenarios
     02 through 07 build on the nodes earlier scenarios created. Do not
     parallelize.

  4. After each agent reports back, apply the oracle.txt checks for that
     scenario against the current braim graph and record the verdict.

  5. Tally PASS / FAIL across the twenty-six scenarios. See oracle.txt SCORING.

DESIGN NOTE

The scenarios are deliberately written as ordinary instructions a working
collaborator might give. The LLM should not be able to infer from the prompt
which braim feature is under test. Whether the resulting graph satisfies the
oracle depends on the LLM's actual behavior under the stated task, not on its
guess about what the operator wants to measure.
