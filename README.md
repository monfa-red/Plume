# Plume

A small, human-readable language for plain-text diagrams. Flex/grid layout when you want it, composable primitives when you don't, CSS-driven theming throughout. Compiles to clean SVG.

```
cat -> dog -> bird
```

A complete diagram: three implicit nodes, two wires, zero ceremony.

See [SPEC.md](SPEC.md) for the language reference and [IMPLEMENTATION.md](IMPLEMENTATION.md) for the build plan.

---

## Features

- **Five sigils, one syntax.** `|type|`, `:` (bind), `.style`, `id.side`, `--var`.
- **Sensible defaults.** Omit the type → `|rect|`. Omit the label → the ID. Reference an undeclared ID in a wire → it auto-creates.
- **13 primitives** (`rect`, `oval`, `hex`, `slant`, `cyl`, `diamond`, `cloud`, `poly`, `path`, `text`, `line`, `icon`, `image`) and **7 templates** (`group`, `badge`, `button`, `card`, `note`, `table`, `cell`).
- **Composable wire ops.** 5 line styles (`-`, `--`, `-.-`, `=`, `~`) × end markers (`<`, `>`, `o`, `<>`): `->`, `<->`, `~o`, `=<>`, `-.->`, …
- **Fan-in / fan-out** with `&` — `a & b -> c` does the obvious thing.
- **Flex + grid layout.** `layout:row|column|(cols,rows)`, with `cell:`, `span:`, `at:`, 9 inner + 8 `out-*` anchors.
- **User-defined shapes** that can carry internal children **and** internal wires that follow you into every instance.
- **Styles** as reusable attribute bundles (`.loud stroke:red thickness:2`).
- **CSS-themable.** Visual defaults emit as live `var()` references; `--bake-vars` inlines them for resvg, librsvg, email.
- **Formatter, linter, live dev server** with SSE auto-reload, all in the same binary.
- **LSP-formatted errors** (`file:line:col: error: …`).

---

## Install & use

```bash
cargo install --path .

plume diagram.plume -o diagram.svg     # compile
plume serve diagram.plume              # live reload in browser
plume fmt --check diagram.plume        # CI-style format check
cat d.plume | plume -                  # stdin
```

### CLI flags

| Flag | Meaning |
|---|---|
| `-o, --output FILE` | Output path (default: stdout). |
| `--format svg\|html` | Raw SVG (default) or wrapped in a minimal HTML page. |
| `--no-defaults` | Omit the default `<style>` block — host page supplies `--plume-*`. |
| `--bake-vars` | Inline `var()` references — required for resvg, librsvg, raster, email. |
| `--theme FILE` | CSS file with `--plume-*` overrides. |
| `--check` | Parse and validate only. |
| `--no-warn` / `--strict` | Silence lint warnings / promote them to errors. |

Exit codes: `0` success, `1` parse/resolve error, `2` I/O, `3` invalid CLI.

---

## Three diagrams, three difficulty levels

### 1. Dead simple

```
dog -> cat
```

Two undeclared IDs become two `|rect|` nodes, connected by a solid arrow.

### 2. A small flow with shapes, styles, and labels

```
{
  |wire| stroke:#444 gap:10
  .loud  stroke:red thickness:2
  |db:cyl| fill:lightyellow
}

api   |rect| "API"
queue |rect| "Queue" radius:8
store |db|   "Postgres"

api   -> queue  "enqueue"
queue -> store  .loud "persist"
store -.-> api  "ack"
```

Defs block sets wire defaults, defines a `.loud` style, and builds a `db` shape from `|cyl|`. Three labelled wires — one styled, one dotted.

### 3. A bit of everything

```
{
  |scene| layout:(3, 2) gap:40 padding:20
  |wire|  stroke:#666 gap:8
  |rect|  radius:4                          // every rect rounds by default

  --accent:#0a84ff
  .loud  stroke:red thickness:2
  .quiet stroke:blue stroke-style:dashed

  |treat:rect| radius:5
  |alert:oval| stroke:red size:36           // circle = oval with scalar size

  |room:group| layout:column gap:8 {
    inlet  |rect| "Inlet"
    outlet |rect| "Outlet"
    inlet -> outlet "flows"                 // internal wire — in every instance
  }
}

cat |oval| "Cat" cell:(1, 1)

kitchen |group| "Kitchen" cell:(2, 1) layout:row gap:10 {
  bowl  |treat| "Bowl"
  water |treat| "Water"
}

garden |group| "Garden" cell:(3, 1) {
  rabbit |alert| "Rabbit" { |badge| "FAST" }
}

closet |room| "Closet" cell:(1, 2)
fridge |room| "Fridge" cell:(3, 2)

cat.r -> bowl & water .loud                 // fan-out, anchored side
closet.outlet -> fridge.inlet "restocks"    // dot-path into instance
kitchen <-> garden .quiet "watches"         // bidirectional, styled, labelled
```

Exercises: scene grid, wire defaults, custom CSS var, styles, two user shapes (one with internal wires), nested groups, anchored endpoints, fan-out, dot-path access, and a styled bidirectional label.

---

## Development

```bash
cargo build
cargo test
cargo run -- samples/hello.plume
cargo run -- serve samples/full_example.plume
```

`samples/` holds one `.plume` per spec feature; `tests/conformance.rs` snapshot-tests them all with `insta`.

## Status

Pre-1.0. Spec is frozen; the compiler ships the pipeline (lex → parse → resolve → layout → render) with 83 passing tests over 25 samples. Wire routing/rendering is being rebuilt — wires parse and resolve, but are not yet drawn.

## License

MIT
