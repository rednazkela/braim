# Graph Viewer Rework

## Status quo

`braim serve` (src/main.rs, `serve_viewer()`) was built to prove nodes were chaining
together — not as a real frontend. It serves a single embedded `viewer.html`
(vis-network, canvas + forceAtlas2 physics) that fetches the *entire* graph as one
JSON dump from `/current.json` and renders it client-side. Physics auto-disables
above 300 nodes (`PHYSICS_AUTO_DISABLE_THRESHOLD`); above that, layout is static
and unorganized. There is no pagination, clustering, level-of-detail, or viewport
culling. Every node and edge in the graph loads and draws, always. Graphs have
since grown large enough that this hangs the browser.

## Goals

- Scale to graphs far larger than today's without the browser choking.
- Smooth interaction (pan/zoom/click), not just a bigger `PHYSICS_AUTO_DISABLE_THRESHOLD`.
- A real color system, not ad hoc per-status colors.
- CLI and UI cross-linked: a node ID printed in the terminal opens straight to that
  node in the browser.

## Architecture

Split server and client. braim's binary stays the API/data server — it already has
the right primitives (`query`, `perspective`, `proximity`, `lookup`, `similar`,
`node`) — the UI becomes a proper build (Vite + TS SPA), compiled into the binary
as static assets so it still ships as one executable.

The client is never handed a full graph dump again. Every view is a bounded
request:

- **Search/landing view** — `lookup`/`similar`, not a rendered graph. Jump straight
  to a node.
- **Node view** — the node plus its N-hop neighborhood via `perspective`/`proximity`,
  filtered by trust/domain server-side before it's sent.
- **Landing/zoom view (revised)** — degree-ranked, viewport-scoped tiles, mirroring
  map tile pyramids. Every node's relevance is its total degree (depends_on count
  plus how many other nodes reference it); the landing screen fetches a high
  min-degree threshold over the full layout extent, rendering the most-connected
  nodes as a real graph — actual nodes and edges, not an aggregate summary. As the
  user zooms in, the client recomputes the current viewport's bounding box (in
  layout-coordinate space) and a continuously-lower min-degree threshold from the
  camera's zoom ratio, fetching and merging in progressively more (and less
  central) nodes scoped to what's actually in view. Domain/type/min-trust filters
  (still present) narrow which nodes are eligible for a tile, composing with the
  degree/viewport scope rather than replacing it. Full-graph rendering never
  happens: the node count in any single tile is bounded by degree threshold and
  viewport, not total graph size.
  (Supersedes the original "cluster summary + click-to-drill" design: clusters
  gave no real graph on landing and their drill-down needed an explicit click
  users didn't discover on their own — replaced here with a scroll/pinch-to-zoom
  interaction closer to how map applications actually work.)

### Rendering

Replace canvas + vis-network with **sigma.js + graphology** (WebGL). Handles tens
of thousands of elements at interactive framerate; has viewport culling and
zoom-based level-of-detail (hide labels/edges below a zoom threshold via
reducers) built in.

Compute layout server-side in Rust, once per graph version, cache it, invalidate
on write. The client places precomputed x/y and pans/zooms — no client-side force
simulation. This removes the 300-node physics cliff by construction rather than
tuning it.

## View / action mapping

Read commands become views; write commands become contextual actions attached to
whatever they act on — not separate editor pages.

| CLI command | UI surface |
|---|---|
| `node <id> --related`, `lookup` | Node detail panel: label, type, domains, sources, verification badge, dependency lists (clickable) |
| `meta <id>` | Fields section inside the node panel |
| `query` / `perspective` / `proximity` | "Connections" mode — pick two nodes/terms, highlight path(s) in place, don't navigate away |
| `why <id>` | Separate causal-chain tab (because_of is a distinct edge type from depends_on) — vertical stepper, consequent → root, each step tagged proven/refuted/untested |
| `similar` | Search bar's "search by meaning" mode |
| `domains` / `list` / `audit` | Left-nav facets + a dedicated Audit worklist (click a finding, jump to its node panel) |
| `statement add-source` | "Add source" button on the node panel |
| `statement invalidate` | "Invalidate" button + reason field on the node panel |
| `statement verify-suggest` | "Suggest sources" — inline candidates, one-click attach |
| `statement contradict` / `resolve-contradiction` | "Mark contested" / "Resolve" on a statement pair |
| `why-add` / `why-test` / `why-remove` | Actions inside the causal-chain tab (drag-connect, pass/fail buttons) |
| `similar --dedup` / `merge-nodes` | "Possible duplicate — merge?" prompt on high-score search hits |
| `import`, `export`, `shard`, `rename-domain`, `init`, `policy`, `version` | Admin-only, not part of the exploration flow (v1 excludes these; a history/diff screen off `version` is worth revisiting later) |

## Color system

Two independent dimensions, encoded separately — don't collapse both into hue.

- **Hue = node_type**, colorblind-safe categorical set (Okabe-Ito): Atomic = blue,
  Compound = purple, Statement/Claim = teal, Fact = green, InvalidStatement =
  vermillion, Source = amber.
- **Fill treatment = verification_status**, independent of hue: unproven = low
  opacity/dashed border; contested = solid amber border override (visible
  regardless of node hue — urgency signal); partial = solid border; proven = full
  opacity; proven_strong = full opacity + thicker border; invalid/refuted =
  diagonal-stripe fill, so it reads in grayscale or to colorblind users — never
  red-hue-only for "invalid."
- **Edges** — neutral gray, thickness/opacity from weight. `contradicts` edges get
  a distinct dashed-red treatment (structurally different from `depends_on`, needs
  to be scannable at a glance).
- **Background** — dark canvas by default (standard for graph viz, better contrast
  for colored nodes); light theme as a toggle.

## CLI ↔ UI linking

Node IDs printed by any command (`lookup`, `query`, `node --related`, `list`,
`audit`, `why`) become clickable in supporting terminals via OSC 8 hyperlink
escape sequences, wrapping just the ID token — not the whole line, so
copy-pasted output for scripts stays clean. Only emitted when stdout is a TTY;
`--json` and piped output stay plain, no escape codes.

**Config: `.env` at project root, `BRAIM_UI_URL`, default `http://localhost:8000`.**
Real env var takes precedence over `.env`, `.env` over the hardcoded default.
Every link-emitting command reads this to build `${BRAIM_UI_URL}/#/node/<id>`.
`braim serve` reads the same variable as its default bind address, so link
generation and the actual server can't drift — one value drives both, `serve
--port` still overrides for one-off runs. Deep-linking `/#/node/<id>` needs no
server-side route fallback (hash routing) and triggers the same bounded
`lookup`/`node --related` fetch the app would make on an in-app click — no
special-casing required, by construction of the bounded-view architecture above.

### Agent operational note

Before referencing a UI link to the user, the assisting agent curl-checks
`BRAIM_UI_URL` (short timeout) and, if unreachable, launches `braim serve` as a
background task rather than telling the user to start it manually. This is
agent-side behavior, not binary logic — the binary doesn't need self-healing
since whoever generates the link can check liveness first.

## Out of scope (v1)

`import`/`export`/`shard`/`rename-domain` admin flows, a version-history diff
screen, auto-opening the browser from the CLI beyond OSC 8 hyperlinks.
