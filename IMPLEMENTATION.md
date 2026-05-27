# Plume — Implementation Plan

`SPEC.md` is the source of truth for what to build. This file is how. It captures the **current state** of the implementation, the **v2 migration** still ahead, and the design decisions worth preserving across sessions.

---

## Goal

A single binary, `plume`, that compiles `.plume` files to SVG per `SPEC.md`. Production-quality base — clean code, modular, no `unsafe`, snapshot-tested, easy to read.

## Architecture

Five-phase compiler pipeline, one module per phase:

```
.plume text → [lex] → tokens → [parse] → AST → [resolve] → resolved IR
                                                              │
                                                              ▼
                                                 [layout] → positioned tree
                                                              │
                                                              ▼
                                                 [route]  → wire paths
                                                              │
                                                              ▼
                                                 [render] → SVG text
```

Each phase is fallible. No panics in the library path — surface invariants as errors. The CLI converts errors to exit codes.

Public API on `lib.rs`:
- `compile_str(&str)`, `compile_str_with(&str, &Options)` → `Result<String, Error>`
- `check_parse`, `check_with` → validation only
- `lint_str` → `Result<Vec<Diagnostic>, Error>`
- `format_source` → canonical reformat
- `serve` → localhost dev server

## Project layout

```
plume/
├── SPEC.md             # source of truth (v2)
├── IMPLEMENTATION.md   # this file
├── src/
│   ├── main.rs         # CLI shim
│   ├── lib.rs          # public API
│   ├── span.rs
│   ├── error.rs        # Error + Diagnostic types, LSP-format display
│   ├── lexer.rs
│   ├── ast.rs
│   ├── parser.rs
│   ├── resolve/
│   │   ├── mod.rs, ir.rs, vars.rs, styles.rs, shapes.rs
│   ├── layout/
│   │   ├── mod.rs, ir.rs, primitives.rs, flex.rs, grid.rs
│   │   ├── anchors.rs, text.rs, values.rs, wires.rs (← routing lives here)
│   ├── render/
│   │   ├── mod.rs, primitives.rs, wires.rs, markers.rs
│   │   ├── values.rs, style_block.rs
│   ├── lint.rs
│   ├── fmt.rs
│   ├── serve/
│   │   ├── mod.rs, playground.html
│   └── theme.rs
├── samples/            # one .plume per spec feature
└── tests/
    ├── conformance.rs  # walk samples/, compile, snapshot SVG
    ├── parsing.rs, resolution.rs, layout.rs (in src), rendering.rs
    ├── fmt.rs, cli.rs, hello.rs
    └── snapshots/      # insta snapshots
```

Split a module when it crosses ~500 LOC. One concept per file.

---

## Current state

The pipeline is on v2 syntax end-to-end and snapshot-tested against 25 samples in `samples/`. What's shipped today:

| Module | Status |
|---|---|
| Lexer / parser | v2 syntax: `\|type\|` for refs, `key:value` attrs (no whitespace around `:`), single defs block led by sigil (`\|scene\|`, `\|name:base\|`, `.style`, `--name:value`), composable wire operators (5 line styles × 5 markers each side), `&` fan, dot-path endpoints with optional `.side`. |
| Resolve | Defs walker (vars / scene config / styles / shapes). Auto-create root rects for unknown wire endpoints. Suffix-match endpoint dot-paths against the scene tree. Cartesian fan expansion. Internal wires in shape defs materialise per-instance with prefixed paths. Visual vs. Layout var-kind split. Cycle / depth-16 inheritance detection. |
| Layout | All 14 primitives. Auto-size to text + `--text-pad`. Per-shape defaults. `layout:row\|column\|(cols,rows)`. `cell:(c,r)`, `span:(c,r)`, `at:`, `offset:`. All 9 inner + 8 `out-*` anchors. Multi-value `padding`/`gap`/`radius`. Rotation. Embedded char-width table. |
| Wire routing | A* on a coarse grid (cell ≈ `--wire-gap` / 2). **Multi-edge candidate selection** (3 of 4 edges per side, picks cheapest). **Endpoint `.side` overrides** pin to a single edge per end, skipping the candidate loop. **Lane fanning** by (full-path, edge): multiple wires exiting the same edge fan to distinct tracks. Obstacle map walks the scene tree — endpoint ancestors are passable, all other shapes are obstacles. 4-tier fallback: shapes+wires → shapes only → no obstacles → straight line. Grid-snapped endpoints (no 1–3 px kinks). |
| Render | Document shell with `@layer plume.defaults`. Per-shape SVG emitters. Shadow filters de-duped in `<defs>`. Auto-classes (`plume-node plume-shape-{type} plume-shape-{parent} plume-style-{name}`). `--bake-vars` for non-CSS renderers. Markers sized `max(--arrow-head=6, thickness × 5)`, tip inset 1 px from shape edge, line shortened 4 px so stroke never pokes past marker. Wire labels with `paint-order=stroke` halo in `--bg` (visually clips the wire under the label). `stroke-style=dashed\|dotted` works on both primitives and wires. |
| CLI | `plume FILE`, `plume fmt`, `plume serve`, `--watch`, `--check`, `--theme`, `--bake-vars`, `--no-warn`, `--strict`, stdin via `-`. Exit codes per SPEC. |
| Linter | One rule shipped: `visual-attr-inline` (fill/stroke/weight/… inline outside a style → warning). Default-on; `--no-warn` silences; `--strict` promotes to error. |
| Formatter | Column-aligned id/type/label/attrs within blank-line-separated groups. Comment + blank-line preservation. Empty-body collapse (`scene {} `). Canonical value emission (numeric normalization, tuple/list spacing). Idempotent. |
| Dev server | `plume serve FILE` — hand-rolled HTTP, SSE auto-reload on file save. No new deps. |

All 86 tests pass (lib + integration + conformance). `cargo clippy --all-targets -- -D warnings` clean. `cargo fmt --check` clean.

---

## v2 migration plan — DONE

The v2 migration is complete. The lexer, AST, parser, resolver, fmt, lint, samples, tests, and snapshots have all been ported. The wire router learned to honour `.side` endpoint overrides. The notes below are kept as historical context for what was changed.

### What is finished

- Sigil-led parsing: `\|type\|`, `\|name:base\|`, `\|scene\|`, `:value`, `.style`, `--name:value`, `&` fan, dot-path endpoints with optional `.side`.
- Single defs block before scene statements; no more `defaults`/`styles`/`shapes`/`scene`/`wires` named blocks.
- Composable wire operators: 5 line styles (`-` `--` `-.-` `=` `~`) × 5 markers each side (none/arrow/crow/dot/diamond).
- Cartesian fan expansion at resolve-time, one ResolvedWire per chain × group product.
- Suffix-match endpoint resolution against scene-tree dot-paths; ambiguity → error suggesting full path.
- Auto-create implicit `\|rect\|`s for unknown wire endpoints, with `label = id`.
- Internal wires in shape defs materialise per-instance with prefixed paths.
- Endpoint `.side` overrides skip the multi-edge candidate loop in the router.
- `plume fmt` emits v2 syntax with column alignment, comment/blank-line preservation, empty-body collapse.
- `visual-attr-inline` lint walks the new AST shape.

### What changed

The SPEC was rewritten to v2 (`|...|` sigils, single defs block, composable wire operators, smart defaults). The implementation is still v1. Migration is essentially a Sprint 1–2 redo — the rest of the pipeline barely cares about syntax.

### What changes

**Lexer** (largest single surface):
- New token: `Pipe` (`|`). Replaces the `Colon` → `Ident` sequence that v1 used for type refs.
- `:` becomes the attr-binding token (was `Equals`). Enforce no whitespace on either side at lex time (or detect at parse time with a clear error message).
- New endpoint-side syntax: `id.side` where `side ∈ {t, b, l, r, top, bottom, left, right}`. The dot here has no whitespace before it. Lex `.` as a single token; parser decides between style-ref (`.alert`, WS-required-before) and endpoint-side (`.r`, no-WS-before).
- New wire-operator characters: `&` (fan), `o` (dot marker — only inside wire ops), `=` (double line), `~` (wavy line). The wire-op lexer should consume the whole compound token (`-o`, `<-o`, `-.->`, `=>`, `~<`, etc.) as one operator token rather than letting the parser try to reassemble them.
- Drop the existing `Equals` token (no more `key=value`). Grandfathering not needed — clean break, samples are being rewritten anyway.

**Parser**:
- Top-level structure changes: optional defs block `{ ... }` first, then scene statements (nodes / wires) at the root. No more named blocks.
- Inside the defs block, dispatch on the leading sigil:
  - `|scene|` → root-scene config (at most one)
  - `|name:base|` → shape def
  - `.name` → style def
  - `--name:value` → var override
- Wire op grammar: parse `[start_marker?][line][end_marker?]` as one token; reject ambiguous mixes within a chain.
- Fan-out / fan-in: `endpoint_group { '&' endpoint }`. Cartesian-expand at resolve.
- Endpoint paths: `ident { '.' ident } ['.' side]`. Resolve as suffix-match against the scene tree post-layout.
- Implicit nodes: a wire referencing an undeclared id creates a `|rect|` at the scene root with the id as label.
- Implicit type: omitted `|type|` defaults to `|rect|`.
- Implicit label: omitted label uses the id.

**AST**:
- `Block` enum collapses to a single `DefsBlock` (replacing the four named blocks) plus a top-level `Vec<Stmt>`. Each defs line is one of `SceneConfig | VarOverride | StyleDef | ShapeDef`.
- `WireOp` widens to a struct `{ line: LineStyle, start: MarkerKind, end: MarkerKind }`. Drop the enum-per-glyph approach.
- `WireEndpoint` gains a `path: Vec<String>` and an `Option<Side>`. Single-segment is the common case.
- `TypeRef` is unchanged shape but populated from `|...|` tokens, not from `Colon`+`Ident`.

**Resolve**:
- The defs-block walker handles var overrides, scene config, style defs, shape defs in order.
- Add an auto-create pre-pass on wires: for every endpoint referenced but not declared, push an empty `|rect|` node into the scene tree at the root with `label = id`. Then run the regular scene-tree resolution.
- Endpoint resolution via suffix-match: walk every scene node's dot-path-from-root and match the wire endpoint against it. Reject ambiguous matches with a helpful error suggesting fully-qualified paths.
- Internal wires in shape defs: instantiate per-instance, scoping IDs under the instance's dot-path.

**Layout, render** — almost no changes. They read AttrMap (still `HashMap<String, ResolvedValue>`) by key name. Attr names stay the same (`size`, `cell`, `points`, `fill`, …); only the SOURCE syntax changes.

**Wire routing** — preserve everything we built. The one v2 addition is **endpoint side override** (`shape.l -> shape.r`): when an endpoint carries a side, the router must use that edge instead of multi-edge A*'s best pick. Plumb the optional side from AST → resolve IR → layout/wires.rs. Skip the edge-candidate loop when a side is forced; route a single A* with the forced start/end edges.

**Linter** — keep the `visual-attr-inline` rule. Sprint 8 expands it (see below).

**Formatter** — emit the new syntax. Rewrite the AST→text printer to use `|type|`, `:` bindings, defs-block-with-leading-sigil. Column alignment logic carries over.

**Samples** — rewrite all 21 to v2. Include at least one example of every new pattern: fan-out (`cat -> dog & bird`), endpoint sides (`cat.r -> dog.l`), auto-created endpoints (`cat -> dog -> bird` with nothing else), implicit types, dot-path navigation into a shape instance, internal wires in a shape def. Re-snapshot.

**Tests** — most of `tests/parsing.rs` is v1-syntax inline. Rewrite the assertions. `tests/cli.rs`, `tests/rendering.rs`, `tests/resolution.rs` likewise. Lib tests in `src/layout/mod.rs` use string-literal Plume too.

### What to KEEP from current code

The pipeline downstream of parse is solid and shouldn't be touched beyond attr-name lookups:

- All of `src/layout/wires.rs` — A*, edge candidates, lane fanning, snap-to-cell, multi-tier fallback. Add `.side` support but don't redesign.
- `src/render/markers.rs` — marker sizing (`max(--arrow-head, thickness × 5)`), tip inset 1 px.
- `src/render/wires.rs` `shorten_for_markers` — line ends 4 px before marker tip. `paint-order` halo on wire labels.
- The `--plume-wire-gap` (16) and `--plume-arrow-head` (6) defaults.
- Grid-cell centering default (h=center v=center on grid containers without explicit alignment).
- `plume fmt` column-alignment logic, comment/blank-line preservation, empty-body collapse.
- `plume serve` SSE auto-reload.
- `--watch` mtime polling.
- The `Diagnostic` infrastructure (level, span, message).

### Sprint 8 — finish the linter

Status: one rule shipped, framework needs expansion. Largely independent of v2 migration but easier to do AFTER v2 (rule logic walks the new AST). Listed here so the next session can pick it up:

- **Rule registry.** Each rule has a stable ID and severity. Diagnostics carry the ID.
- **New rules** (one file each under `src/lint/`):
  - `unused-style`, `unused-shape`, `unused-default` — declared but never referenced.
  - `duplicate-style-ref` — `.a .a` on one inst.
  - `wire-self-loop` — `a -> a` (warn before resolve so authors see it earlier).
  - `gap-without-layout` — `gap:` on a container with no `layout:` attr.
  - `ambiguous-auto-created-endpoint` — wire endpoint that resolves to an auto-created node (catches typos like `cta` vs `cat`).
- **Configurable severity.** `--allow rule` / `--deny rule` CLI flags. Inline `// plume-allow: rule-id`.
- **Autofix.** `plume lint --fix` rewrites source for fixable rules (use `src/fmt.rs` to emit).
- **JSON output.** `plume lint --json` for LSP / CI integrations.
- **Diagnostic ranges.** Widen `Diagnostic.span` from a point to a range.

### Syntax highlighting & editor support

Once v2 lexer is stable, add:

- A TextMate grammar (`plume.tmLanguage.json`) for VS Code / GitHub syntax highlighting.
- Optionally an LSP server (Sprint 9 candidate) — the parser already emits spans, and `lint_str` already returns structured diagnostics. The pieces are mostly there; missing is the `tower-lsp` shell.

### Suggested order

1. Lex + parse v2 → existing samples don't compile yet.
2. Update resolve (defs block, auto-create, dot-paths).
3. Rewrite samples; re-snapshot.
4. Add `.side` endpoint override in routing.
5. Update fmt + lint to v2.
6. Sprint 8 expansion.
7. Syntax highlighting / LSP.

Steps 1–3 are the bulk. 4 is small. 5 is moderate. 6–7 are independent follow-ups.

---

## Locked decisions

- Single crate (split later only if WASM/LSP demand it).
- Hand-written recursive-descent parser. LL(1) per SPEC §17.
- Hand-written SVG output via typed builder helpers. Zero deps for output.
- No `unsafe` anywhere.
- `clap` (derive) for CLI args.
- `insta` for snapshot tests.
- LSP-format errors via plain `Display` impls.
- Material Symbols bundled at build time via `build.rs` (TODO — currently a stub).
- Embedded char-width table for default font.

## Open decisions

| Question | Notes |
|---|---|
| Icon table shape | `phf::Map`, sorted slice + bsearch, or `match`. Pick when implementing `render/icons.rs`. |
| Color parsing | Hand-write (~50 LOC) vs. `csscolorparser`. Lean hand-write. |
| Theme CSS parsing | Hand-write line scanner vs. `cssparser`. Hand-write probably wins. |
| Icon set scope | All ~3000 Material Symbols vs. curated subset. Tree-shake regardless. |
| Error pretty-printer | Plain LSP vs. `ariadne` / `miette`. Stay plain unless feedback demands richer output. |

## Dependencies

Locked:

```toml
clap = { version = "4", features = ["derive"] }
insta = { version = "1", features = ["glob"] }  # dev
```

Avoid unless justified: `serde`, `tokio`, parser combinators, XML libs.

---

## Testing strategy

1. **Unit tests per module** — pure logic (bbox math, color parsing, anchor resolution).
2. **Snapshot tests** via `insta` — phase outputs and final SVG. `cargo insta review` to accept changes.
3. **Conformance** — `tests/conformance.rs` walks `samples/`, compiles each, snapshots SVG.
4. **Visual checks** — for layout-sensitive samples, render to PNG via `resvg` CLI and inspect (the multimodal Read tool handles this).
5. **Idempotence** — `plume fmt` snapshot tests assert `fmt(fmt(x)) == fmt(x)`.

CI runs everything on push.

## Visual verification loop

```
1. Edit code.
2. cargo run -- samples/<sample>.plume --bake-vars -o /tmp/out.svg
3. resvg /tmp/out.svg /tmp/out.png
4. View /tmp/out.png (in a viewer or via the multimodal Read tool).
5. Iterate.
```

`--bake-vars` is required for `resvg`/`librsvg` — they ignore CSS custom properties, so a spec-conformant SVG with `fill="var(--plume-fill)"` renders black-on-black. Browsers handle the live form fine.

Faster loop: `plume serve samples/foo.plume` opens an auto-reloading page at `http://127.0.0.1:7700/`.

## Build / run commands

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt

cargo run -- samples/hello.plume                          # to stdout
cargo run -- samples/hello.plume -o /tmp/hello.svg
cargo run -- --check samples/hello.plume                  # validate only

cargo run -- fmt samples/hello.plume                      # rewrite in place
cargo run -- fmt --check samples/hello.plume              # CI mode

cargo run -- serve samples/hello.plume                    # localhost preview
cargo run -- samples/hello.plume --watch -o /tmp/out.svg  # recompile on save

cargo insta review                                        # accept snapshot changes
```

## Notes on style

- One concept per file. Split when crossing ~500 LOC.
- Public API on `lib.rs` is small. Everything else is `pub(crate)` or private.
- Errors implement `Display` in LSP format.
- Prefer `match` over chains of `if let`. Prefer named structs over tuples for anything with > 2 fields.
- Default to no comments. Comments only for non-obvious *why*. Never explain *what* — rename instead.

---

## Future (post v2)

- LSP server. Parser emits spans; `lint_str` returns structured diagnostics — most of the work is a `tower-lsp` shell.
- WASM target. Needs the workspace split (`plume-core` + `plume-cli`).
- mdbook docs site under `docs/`.
- Auto-layout via a graph library (force-directed, Sugiyama). Currently in SPEC §20 non-goals.
- Real Material Symbols icon embedding (build.rs scanning `assets/icons/`). Currently a placeholder.
- Rounded wire corners (replace orthogonal-corner `L` segments with quarter-arcs).
- Manual wire waypoints.
