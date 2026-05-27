# Plume — Language Specification (v2)

A small, human-readable language for plain-text diagrams. Flex/grid layout, composable primitives, CSS-driven theming. Compiles to clean SVG.

This document is complete — an implementer should be able to build a conforming engine from it alone.

---

## Table of Contents

1. [Mental Model](#1-mental-model)
2. [File Format & Lexical Syntax](#2-file-format--lexical-syntax)
3. [Sigils — Reference](#3-sigils--reference)
4. [The Defs Block](#4-the-defs-block)
5. [Node Declarations](#5-node-declarations)
6. [Layout](#6-layout)
7. [Positioning & Anchors](#7-positioning--anchors)
8. [Built-in Primitives](#8-built-in-primitives)
9. [Built-in Templates](#9-built-in-templates)
10. [Wires](#10-wires)
11. [Attribute Reference](#11-attribute-reference)
12. [Variables & Defaults](#12-variables--defaults)
13. [Specificity / Application Order](#13-specificity--application-order)
14. [SVG Output](#14-svg-output)
15. [CLI](#15-cli)
16. [Errors](#16-errors)
17. [Formal Grammar (EBNF)](#17-formal-grammar-ebnf)
18. [Implementer Algorithm](#18-implementer-algorithm)
19. [Reserved Words](#19-reserved-words)
20. [Non-Goals (v2)](#20-non-goals-v2)
21. [Complete Example](#21-complete-example)

---

## 1. Mental Model

A Plume file is **one optional defs block followed by the scene**:

```
{                  // optional, must be first if present
  |scene|         layout:row gap:30      // configure root scene (one line, max)
  --gap:24                               // var overrides
  .alert          stroke:red             // style defs
  |my_psu:rect|   radius:5               // shape defs
}

// scene nodes and wires at the root, in any order
motor  |my_psu| "Motor"
driver |my_psu| "Driver"
motor -> driver "24V"
```

**Five sigils, each with one or two clearly-disambiguated jobs:**

| Sigil | Meaning |
|---|---|
| `\|name\|` | Type reference (built-in or user-defined shape). |
| `\|name:base\|` | Shape definition — inside the defs block only. |
| `:` | Binds left to right. Used for attrs (`radius:5`) and inheritance (`my_a:rect`). Never surrounded by whitespace. |
| `.name` | Style reference (whitespace-before required). |
| `id.side` | Endpoint side on a wire (no whitespace). |
| `--name` | CSS variable reference. |

**Defaults that make small diagrams trivial:**

- Omitting `\|type\|` defaults to `\|rect\|`.
- Omitting the label uses the node's ID as its label (`""` to suppress).
- Referencing an undeclared ID in a wire creates an implicit `\|rect\|` node at the scene root.

So this is a complete diagram:

```
A -> B -> C
```

**One pass, no forward references** for explicit declarations: a style or shape used in the scene must be defined above its first use in the defs block. Wire-to-undeclared-id auto-creates, but explicit re-declaration after auto-creation is an error.

**Nothing is hardcoded.** Every default the engine uses (colors, fonts, sizes, padding, gaps) is a named CSS variable. See [§ 12](#12-variables--defaults).

---

## 2. File Format & Lexical Syntax

| Property | Value |
|---|---|
| Extension | `.plume` |
| Encoding | UTF-8 (BOM ignored) |
| Line endings | LF or CRLF (CRLF normalized on read) |
| Comments | `// ...` to end of line. No block comments. |
| Statement ends | newline or `;`. |
| Identifier | `[a-zA-Z_][a-zA-Z0-9_-]*` — case-sensitive, ASCII only. |

Whitespace is insignificant except as a token separator and where called out by sigil rules below.

### Sigil whitespace rules

| Form | Rule |
|---|---|
| `\|...\|` | Opening `\|` and closing `\|` paired. Whitespace allowed inside; no whitespace allowed at the boundary if next/prev token is an ident (see node decl). |
| `:` (binding) | **No whitespace before or after.** `radius:5`, `my_a:rect`. `radius: 5` is a syntax error. |
| `.name` (style ref) | **Whitespace required before** when following an identifier or closing `\|`. `drive .alert` ✓ ; `drive.alert` is parsed as endpoint side. |
| `id.side` (endpoint side) | **No whitespace before.** Only valid in wire endpoints. |
| `--name` (var ref) | Either appears as an attr value or — in defs — at line start to override a var. |

### Strings

Double-quoted UTF-8. Escapes: `\"`, `\\`, `\n`, `\t`. Single quotes are not strings.

### Numbers

Integer or decimal, optional sign. No units (px for lengths, degrees for angles, 0–1 for opacities).
Examples: `10`, `-5`, `0.25`, `+3`, `123.456`.

### Tuples & Lists

| Form | Example |
|---|---|
| Tuple (2–5 components) | `(10, 20)`, `(2, 2, 4, gray)` |
| List | `[(0,0), (10,10), (20,5)]` |

The component count is determined by the receiving attr.

### Colors

`#fff`, `#ffaa00`, `#ffaa00cc` (alpha), CSS named colors (`red`, `cornflowerblue`), `rgb(...)`, `rgba(...)`, `hsl(...)`, `--name` (Plume CSS-var reference, see [§ 12](#12-variables--defaults)), or `none`. Out-of-range channel components are an error.

---

## 3. Sigils — Reference

The complete sigil table. Each row is a parsing rule.

| Form | Where it appears | Means |
|---|---|---|
| `\|rect\|` | Anywhere a type is expected | Reference to built-in or user-defined type. |
| `\|psu:rect\|` | Defs block only | Define shape `psu`, base `rect`. |
| `\|scene\|` | Defs block only (one max) | Configure the root scene container. |
| `key:value` | After type/styles, in attr lists | Attribute binding. |
| `name:base` | Inside `\|...\|` in defs | Inheritance binding. |
| `.alert` | After type, with WS before | Apply style `alert`. |
| `a.r` | Wire endpoint (no WS before) | Endpoint at side `r` of node `a`. |
| `--gap` | Attr value, or defs line start | Reference / override CSS variable `--plume-gap`. |

The `:` and `.` sigils each carry two jobs disambiguated by whitespace and grammatical position. Type refs (`\|...\|`) always carry exactly one job.

---

## 4. The Defs Block

A file may begin with **one** anonymous block `{ ... }`. It contains, in any order:

```
{
  // scene config — at most one `|scene|` line
  |scene| layout:(3,2) gap:40 padding:20 background:--bg

  // CSS var overrides — leading `--`
  --gap:24
  --accent:#0a84ff

  // styles — leading `.`
  .alert  stroke:red thickness:2
  .ghost  opacity:0.3

  // shape definitions — `|name:base|`
  |psu:rect|     radius:5
  |bus:slant|    fill:gray
  |panel:group| layout:column gap:8 padding:12 {
    |text| "Title" at:top weight:bold
    body |text| "Content"
  }
}
```

The leading sigil tells the parser what kind of line it is:

| First token | Line kind |
|---|---|
| `\|scene\|` | Scene config (at most one per file). |
| `\|name:base\|` | Shape def. |
| `.name` | Style def. |
| `--name` | Var override. |

If no defs block is needed, omit it. Scene nodes start directly at the top of the file.

### Scene config (`|scene|`)

Configures the root scene container — `layout`, `gap`, `padding`, `background`, `h`, `v`, `col-widths`, `row-heights`, etc. See [§ 6](#6-layout). At most one `|scene|` line per file.

**Default when omitted:** `layout:row gap:--gap padding:--canvas-pad`. Quick diagrams like `A -> B -> C` lay out left-to-right without ceremony.

### Style defs

`.name attrs…` — a reusable attribute bundle. May reference other styles by `.other` (applied left-to-right). Cycles are an error.

### Shape defs

`|name:base| attrs… { body }` — define a new type. The base may be any built-in primitive, built-in template, or previously-defined user shape. **At least one of attrs or body must be present** (or the def has no effect). Max inheritance depth: **16**. Cycles are an error.

A shape body may contain ID'd children **and** internal wires that reference those IDs (see [§ 10](#10-wires)). Internal IDs are scoped to the body.

---

## 5. Node Declarations

The full form:

```
id [|type|] ["label"] ["href"] [.style…] [attrs…] [{ body }]
```

Every part except `id` is optional.

| Form | Effect |
|---|---|
| `motor` | Implicit `\|rect\|`, label = "motor". |
| `motor \|psu\|` | Shape `psu`, label = "motor" (ID-derived). |
| `motor "Motor 960W"` | Implicit `\|rect\|`, label = "Motor 960W". |
| `motor \|psu\| ""` | Shape `psu`, **no** label. |
| `motor \|psu\| "Motor" "https://example.com"` | Label + clickable: whole shape wrapped in `<a href>`. |
| `motor \|psu\| .bold .alert cell:1 padding:5` | Shape + styles + attrs. |
| `panel \|group\| { … children … }` | Container with body. |

**Order on the line is strict:** id → type → label → href → styles/attrs (may interleave) → `{ body }`.

### Inside a container body

Primitives may be anonymous (no id). They're declared starting with `|type|`:

```
panel |group| {
  |text| "Title" at:top weight:bold
  body |text| "Content"
}
```

Without an id, a primitive can't be referenced from a wire. Give it an id to make it wire-addressable.

### Implicit declarations

A wire that references an undeclared id auto-creates an empty `|rect|` at the scene root with the id as its label:

```
Motor -> Driver       // creates Motor and Driver as |rect|s
```

If you later explicitly declare an auto-created id, that is a duplicate-id error. **To customize, declare explicitly *before* first use.**

### Label sugar

`id |type| "label"` expands to a `:text` child:

```
motor |psu| "Motor 960W"
// equivalent to:
motor |psu| { |text| "Motor 960W" }
```

If both sugar and explicit `|text|` children are present, the sugar's text is added first.

Multi-line labels: use `\n`. The text bbox sizes to the widest line; vertical spacing is `size × 1.2`.

### href (link)

A second positional string after the label becomes the node's `href`. The whole shape is wrapped in `<a href>` — every child becomes part of the clickable region. On a `|text|` primitive, only that text is wrapped.

---

## 6. Layout

Any container picks a layout mode via the `layout` attr:

| Value | Behavior |
|---|---|
| `layout:row` | 1D horizontal flex — children flow left-to-right. |
| `layout:column` | 1D vertical flex — children flow top-to-bottom. |
| `layout:(COLS, ROWS)` | 2D grid with `COLS` columns and `ROWS` rows. |

For grid containers, child cells use `cell:(c, r)` to place into a specific track and `span:(c, r)` to span multiple tracks. Both use **(col, row)** = **(x, y)** order.

### Container attrs

| Attr | Applies to | Notes |
|---|---|---|
| `layout` | all | `row`, `column`, or `(c, r)` tuple. |
| `gap` | all | Spacing between children. Scalar = both axes; `(y, x)` = vertical / horizontal. Negative allowed. |
| `padding` | all | Inner padding. `N`, `(y, x)`, or `(t, r, b, l)`. |
| `col-widths`, `row-heights` | grid | Fixed track sizes. Scalar = all equal; list = explicit per track. |
| `h`, `v` | all | Axis alignment / distribution. |
| `background` | scene only | Canvas background color. |

When `col-widths` / `row-heights` are set, cells take exactly those sizes (children with explicit `size:` still override). Omitted → grid auto-sizes to the widest/tallest child in each track.

### Multi-value `padding`, `radius`

| Form | Meaning |
|---|---|
| `N` | All four sides |
| `(y, x)` | Vertical, horizontal |
| `(t, r, b, l)` | Clockwise from top |

For `radius`: 2-val = `(top-corners, bottom-corners)`.

### `h:` and `v:` values

| Value | Stacking axis | Cross axis |
|---|---|---|
| `start`, `center`, `end` | Pack at edge / centered / opposite | Align child to edge / center / opposite |
| `stretch` | (no effect) | Children fill the cross axis |
| `between`, `around`, `evenly` | Distribute | (treated as `start`) |

For `layout:row`, stacking is horizontal; for `layout:column`, vertical; for grids, both axes are stacking and `h` / `v` align cell content.

**Defaults.** Grid cells: `h:center v:center`. Flex containers: `start` on stacking axis, `stretch` on cross axis (CSS Flexbox-style). Root scene (when `|scene|` is omitted from defs): `layout:row gap:--gap padding:--canvas-pad`.

### Child positioning

| Attr | Effect |
|---|---|
| `at:(x, y)` | Place child's center at (x, y). Removes from flow. |
| `at:anchor` | Named anchor — see [§ 7](#7-positioning--anchors). |
| `offset:(x, y)` | Fine-tune from an anchor. |
| `cell:(c, r)` | Grid track placement, 1-indexed. |
| `span:(c, r)` | Grid track span. Default `(1, 1)`. |
| `z:N` | Render-order override. Source order is the tiebreak. |

`at:` always beats `cell:`. An absolutely-positioned child still contributes to the parent's bbox. Out-of-range cell coordinates are an error.

---

## 7. Positioning & Anchors

A shape's **bounding box** is the smallest axis-aligned rectangle that fully contains it, including its stroke.

### Positioning rules

1. **Center origin.** Every shape's bbox is centered at the parent's coordinate origin by default. `at:(x,y)` puts the bbox center at (x,y).
2. **`origin:top-left`** opts into CSS-style top-left positioning per instance (or globally via defs).
3. **Source order = render order.** Later renders on top. `z:N` overrides; ties broken by source order.
4. **Strokes count toward bbox** — `|rect| size:(100, 50) thickness:4` has bbox 104×54.
5. **`|path|` is the only exception to center-origin** — `d:` uses native SVG top-left coordinates.
6. **Rotation** applies last as an SVG transform; the rotated bounding rectangle propagates up the tree.

### Anchors

Bare names that resolve to positions relative to the parent's bbox.

**Inside the parent:** `center`, `top`, `bottom`, `left`, `right`, `top-left`, `top-right`, `bottom-left`, `bottom-right`.

**Outside the parent** (places the child's facing edge tangent to the parent's): `out-top`, `out-bottom`, `out-left`, `out-right`, plus the four corner variants.

`offset:(x,y)` shifts from any anchor. **Out-\* anchors are computed against the parent's bbox excluding out-\* children** — preventing infinite recursion.

**Wire-route anchors** (only valid on a `|text|` child of a wire): `start`, `mid`, `end`, or a fractional number `0..1` along the route.

### Auto-sizing

Closed shapes auto-size to their text children + `--text-pad` (default 16) on each side when `size:` is omitted. Text bbox width comes from embedded font metrics.

If neither `size:` nor text is given:

| Shape | Default `size` |
|---|---|
| `\|rect\|`, `\|group\|`, `\|slant\|` | `(100, 40)` |
| `\|oval\|` | `(60, 40)` |
| `\|hex\|`, `\|cyl\|`, `\|diamond\|`, `\|cloud\|` | `(60, 60)` |
| `\|icon\|` | `24` |
| `\|poly\|`, `\|image\|` | Error if required attrs missing |

`|line|` and `|arrow|` always require explicit `points:[…]`.

---

## 8. Built-in Primitives

14 primitives. All accept position attrs and visual style attrs; closed shapes also accept `double:`, `rotation:`, `shadow:`.

**Dimension attrs unified.** All closed shapes use `size:`:
- `size:N` — square / circle (single value, applied to both axes)
- `size:(w, h)` — rectangle / ellipse

`size:` semantics are **bbox dimensions**: `|oval| size:(60, 40)` produces an ellipse occupying a 60×40 box (rx=30, ry=20 internally).

| Primitive | Required | Notes |
|---|---|---|
| `\|rect\|` | `size` (auto) | Rounded corners via `radius:`. |
| `\|oval\|` | `size` (auto) | Bbox-based ellipse. Use `\|circle\|` template for `size:N` sugar. |
| `\|hex\|` | `size` (auto) | Regular hex, flat top/bottom. |
| `\|slant\|` | `size` (auto) | Parallelogram, top edge shifted by `tan(skew) × h`. `skew` in degrees, range (-89, 89). |
| `\|cyl\|` | `size` (auto) | Cylinder. Body height = `h`; top/bottom ellipses extend ±h/8. |
| `\|diamond\|` | `size` (auto) | Rhombus inscribed in the bbox. |
| `\|cloud\|` | `size` (auto) | Stylized cloud, fixed path template scaled to fit. |
| `\|poly\|` | `points:[(x,y),…]` | ≥3 points. Local coords (center-origin). Closed shape. |
| `\|path\|` | `d:"..."` | Raw SVG path. **Native top-left coords** (only exception). |
| `\|text\|` | label string | See [§ 5 label sugar](#label-sugar) and [§ 11 text attrs](#text). |
| `\|line\|` | `points:[(x,y),…]` | 2+ points. Markers via `marker*:` attrs. |
| `\|arrow\|` | `points:[(x,y),…]` | A `\|line\|` with `marker-end:arrow` by default. |
| `\|icon\|` | `name` | Material Symbols. `variant:outlined\|filled\|rounded\|sharp`, `size:N`. Compiler bundles only referenced icons. |
| `\|image\|` | `href size` | Emits `<image href="...">`. External URLs only; no embedding. |

### Visual modifiers

Apply to closed shapes:

| Attr | Forms | Effect |
|---|---|---|
| `stroke-style` | `solid` / `dashed` / `dotted` | Stroke pattern. Default `solid`. |
| `double` | `N` / `(x, y)` | Render twice with offset, second copy behind. Scalar uses `(N, -N)`. |
| `rotation` | `N` degrees | Rotate around bbox center. |
| `shadow` | `N` / `(dx, dy)` / `(dx, dy, blur)` / `(dx, dy, blur, color)` | Drop shadow via SVG `<filter>`. |

### Markers (on `|line|`, `|arrow|`, and wires)

| Attr | Effect |
|---|---|
| `marker:X` | Shorthand: both ends. |
| `marker-start:X` | Start end (or wire source). |
| `marker-end:X` | End end (or wire target). |

Values: `none`, `arrow`, `dot`, `diamond`, `crow`. Markers scale linearly with thickness, floor at `--arrow-head`.

**Per-type defaults:**

| Type | start | end |
|---|---|---|
| `\|line\|` | none | none |
| `\|arrow\|` | none | arrow |
| Wires | derived from operator (see [§ 10](#10-wires)) |  |

Source-order wins on conflicts: `marker:arrow marker-end:dot` → start=arrow, end=dot.

Marker color = stroke color.

---

## 9. Built-in Templates

Each template is an attribute bundle over a primitive base.

| Template | Base | Defaults | Use for |
|---|---|---|---|
| `\|group\|` | `\|rect\|` | `stroke-style:dashed stroke:--muted fill:none padding:15`; text `at:top weight:bold` | Frame + label slot. |
| `\|circle\|` | `\|oval\|` | `size:40` default. `size:N` makes a circle of diameter N. | Convenience for circles. |
| `\|badge\|` | `\|rect\|` | `at:top-right radius:999 padding:(2, 8) shadow:2 fill:--accent z:10`; text small + on-accent | Floating pill on a parent's corner. |
| `\|button\|` | `\|rect\|` | `radius:4 padding:(8, 16) shadow:2 fill:--accent`; text on-accent | Use with link to click. |
| `\|card\|` | `\|rect\|` | `radius:8 padding:16 shadow:2 stroke:none fill:--fill` | Content surface, no border. |
| `\|note\|` | `\|rect\|` | `radius:2 padding:12 shadow:2 stroke:none fill:--note-bg` | Sticky-note look. |
| `\|db\|` | `\|cyl\|` | alias | Database, friendlier name. |
| `\|table\|` | `\|group\|` | Use with `layout:(c, r)`, `col-widths:`, `row-heights:`, `gap:0`, `stroke:none` | Container for `\|cell\|`s. |
| `\|cell\|` | `\|rect\|` | `padding:8 stroke:--stroke thickness:1 fill:none` | Bordered cell. |
| `\|dim\|` | `\|line\|` | `marker:arrow` (both ends) | Dimension line. Add `\|text\| at:center` child for the label. |

Templates can be extended in the defs block like any user shape.

---

## 10. Wires

Wires connect scene-node IDs.

### Operator grammar

A wire op is `[start_marker?][line][end_marker?]`, no spaces:

| Part | Tokens |
|---|---|
| Line | `-` solid, `--` dashed, `-.-` dotted, `=` double, `~` wavy |
| Markers (start side) | `<` (arrow), `>` (crow), `o` (dot), `<>` (diamond) |
| Markers (end side) | `>` (arrow), `<` (crow), `o` (dot), `<>` (diamond) |

The same character means different markers at start vs end (`<` at start = arrow, `<` at end = crow). Position discriminates.

**Common operators:**

| Op | Markers | Line |
|---|---|---|
| `->` | none / arrow | solid |
| `<-` | arrow / none | solid |
| `<->` | arrow / arrow | solid |
| `-o` | none / dot | solid |
| `o-` | dot / none | solid |
| `o-o` | dot / dot | solid |
| `-<>` | none / diamond | solid |
| `<>-<>` | diamond / diamond | solid |
| `-<` | none / crow | solid |
| `>-<` | crow / crow | solid |
| `o->` | dot / arrow | solid |
| `<-o` | arrow / dot | solid |
| `-->` `--o` `--<>` `--<` | (same, dashed) | dashed |
| `-.->` `-.-o` `-.-<>` `-.-<` | (same, dotted) | dotted |
| `=>` `=o` `=<>` `=<` | (same, double) | double |
| `~>` `~o` `~<>` `~<` | (same, wavy) | wavy |
| `-` `--` `-.-` `=` `~` | no markers | (each style) |

**Wire defaults.** If an operator carries no markers, it has none on both ends. Explicit `marker:`, `marker-start:`, `marker-end:` attrs override the operator (source-order wins per [§ 8](#markers-on-line-arrow-and-wires)).

### Wire syntax

```
endpoints op endpoints [op endpoints …] ["label"] [.style…] [attrs…] [{ children }]
```

`endpoints` is one or more endpoints joined by `&` (fan-out/fan-in):

```
a -> b               // 1 wire
a -> b -> c          // chain: 2 wires
a -> b & c           // fan-out: 2 wires (a→b, a→c)
a & b -> c           // fan-in: 2 wires
a & b -> c & d       // cartesian: 4 wires
a -> b -> c & d      // chain + fan: a→b, b→c, b→d
```

Mixing operators within one chain is a parse error. Children may only be `|text|` (wire labels).

### Endpoints

An endpoint is a dot-path with an optional side suffix:

```
endpoint    = ident { '.' ident } [ '.' side ]
side        = 't' | 'b' | 'l' | 'r' | 'top' | 'bottom' | 'left' | 'right'
```

The parser treats the final segment as a side iff it matches a reserved side name. Otherwise it's the last path segment.

**Resolution (suffix-match):** the resolver finds the scene tree node whose path matches the endpoint's path. A single-segment path `motor` matches any scene node whose **last path segment** is `motor`. If the match is ambiguous, qualify with more segments: `rails.rail48.drive`. Full dot-paths from scene root always match exactly.

| Endpoint | Resolves to |
|---|---|
| `motor` | scene node whose path ends in `.motor` (unique) |
| `auth.db` | path ends in `auth.db` (unique) |
| `motor.r` | scene node `motor`, right edge |
| `auth.db.l` | scene node `auth.db`, left edge |

Without a side, the router picks entry / exit edges by relative geometry. With a side, that edge is forced.

### Label sugar

`a -> b "label"` expands to `a -> b { |text| "label" at:mid }`. For chains and fans, the label sits at the midpoint of the overall route.

### Wire-text children

```
a -> b {
  |text| "label" at:mid size:10
  |text| "↓"     at:0.75
}
```

`at:` accepts wire-route anchors only (`start`, `mid`, `end`, or `0..1`). `offset:(x,y)` shifts in the route's local tangent frame.

### Internal wires in shape definitions

A shape body may contain wires that reference its internal children:

```
{
  |microservice:group| layout:column gap:10 {
    api |rect| "API"
    db  |cyl|  "DB"
    api -> db "queries"
  }
}

auth  |microservice| "Auth Service" cell:1
users |microservice| "User Service" cell:2

auth.db -> users.api "replicates"
```

IDs inside the body are local. On instantiation, internal wires materialize within each instance's subtree. From outside, the dot-path navigates: `auth.db`, `users.api`.

### Routing

Wires route orthogonally on a coarse grid using A* with bend penalty. The router picks entry / exit edges by relative geometry unless overridden by an explicit side.

**Obstacle rules.** Routes clear other shapes by at least `--wire-gap` (default 16). Wires also try to stay `--wire-gap` from other wires.

| Shape | Treated as |
|---|---|
| Wire's source or target | Endpoint — not an obstacle |
| A group that contains the source or target | Passable — recurse into its children |
| Any other shape, including groups | Hard obstacle |

**Fallback hierarchy.** The router tries each tier and stops at the first that succeeds:

1. Path that respects gap from all shapes and wires.
2. Path that crosses other wires.
3. Path that crosses shapes (when fully surrounded).
4. Straight line from edge to edge.

Markers are inset 1 px from their endpoint.

**Self-loops** (`a -> a`): a small loop exits the right edge, curves over the top, re-enters the top edge (diameter = `--rect-h × 0.75`).

**Duplicate / parallel wires** between the same pair fan out: entry / exit points offset by `gap` along the edge.

Manual waypoints are not in v2.

---

## 11. Attribute Reference

Every attr has the form `name:value`. No bare attrs.

### Visual (style)

Putting these inline (outside the defs block) emits a lint warning — see [§ 16](#16-errors).

| Attr | Type | Default |
|---|---|---|
| `fill` | color | `--fill` (closed shapes), `--text-color` (text), `--stroke` (icons) |
| `stroke` | color | `--stroke` |
| `thickness` | number | `--thickness` (1) |
| `stroke-style` | `solid` / `dashed` / `dotted` | `solid` |
| `opacity` | 0..1 | 1 |
| `radius` | scalar / (y, x) / (t, r, b, l) | `--radius` (0) |
| `double` | `N` / `(x, y)` | off |
| `rotation` | degrees | 0 |
| `shadow` | `N` / `(dx, dy)` / `(dx, dy, blur)` / `(dx, dy, blur, color)` | off |
| `marker`, `marker-start`, `marker-end` | marker name | per-type |

### Geometry

| Attr | Type | Notes |
|---|---|---|
| `at` | `(x, y)` or anchor | bbox center at (x, y). |
| `offset` | `(x, y)` | From anchor. |
| `size` | `N` or `(w, h)` | Bbox dimensions. Scalar = square / circle. |
| `points` | `[(x, y), …]` | Vertex list. |
| `d` | string | Raw SVG path data (`\|path\|` only). |
| `skew` | number | Slant degrees (`\|slant\|` only). |
| `origin` | `center` / `top-left` | Bbox origin. |
| `z` | integer | Render-order. |

### Container & grid

`layout`, `gap`, `padding`, `col-widths`, `row-heights`, `h`, `v`, `background`, `cell`, `span` — see [§ 6](#6-layout).

### Text

| Attr | Default | Notes |
|---|---|---|
| `at` | `center` | Anchor or `(x, y)`. |
| `align` | `center` | `left` / `center` / `right` — multi-line alignment. |
| `size` | `--text-size` (13) | Font size, px. |
| `weight` | `normal` | `normal` / `bold`. |
| `fill` | `--text-color` | Text color. |
| `fit` | `none` | `none` / `shrink` / `wrap` / `clip` — overflow behavior. |

No per-node font attr — Plume enforces one font per diagram via `--font`.

### Accessibility & interaction

| Attr | Notes |
|---|---|
| `title` | `<title>` child — browser tooltip + screen reader. |
| `aria-label` | Emitted on the `<g>`. |

Links: use the positional second string after the label (see [§ 5](#5-node-declarations)).

---

## 12. Variables & Defaults

All defaults live in CSS variables. Override at any level:

```
built-in fallback → --theme file → defs `--name:value` → runtime CSS (visual only)
```

**Layout variables** (gap, padding, dimensions) bake at compile time. **Visual variables** (colors, fonts) emit as live `var(--plume-*)` references and respond to runtime CSS.

### 12.1 Built-in CSS variables

```
Visual (live at runtime):
  --plume-bg            white
  --plume-fg            #222
  --plume-fill          white
  --plume-stroke        #444
  --plume-accent        #0a84ff
  --plume-on-accent     white
  --plume-muted         #888
  --plume-danger        crimson
  --plume-warn          orange
  --plume-note-bg       #fff9c4
  --plume-font          system-ui, -apple-system, sans-serif
  --plume-text-color    var(--plume-fg)
  --plume-shadow        rgba(0, 0, 0, 0.2)

Layout (compile-time):
  --plume-text-size     13
  --plume-text-pad      16
  --plume-gap           20
  --plume-padding       0
  --plume-thickness     1
  --plume-radius        0
  --plume-rect-w        100
  --plume-rect-h        40
  --plume-oval-w        60
  --plume-oval-h        40
  --plume-circle-size   40
  --plume-arrow-head    6
  --plume-icon-size     24
  --plume-canvas-pad    20
  --plume-wire-gap      16
```

### 12.2 `--name` references in attribute values

Any value of the form `--name` is a Plume CSS-variable reference. The compiler:

1. Prepends `--plume-` to form the full CSS variable name.
2. For **visual** vars, emits `var(--plume-name)` so runtime CSS can theme it.
3. For **layout** vars, bakes the compile-time value into the SVG.

To use a non-Plume CSS variable, alias it via host CSS:

```css
.plume { --plume-accent: var(--my-brand-blue); }
```

### 12.3 `@layer plume.defaults`

In standalone mode the embedded `<style>` wraps default variables in `@layer plume.defaults { ... }`. Any unlayered host CSS automatically wins, no `!important` needed.

---

## 13. Specificity / Application Order

For any node, wire, or primitive, attrs merge in this order — **later wins**:

1. **Type defaults** (and parent types, recursively).
2. **Style classes** — applied left-to-right.
3. **Inline attrs** — `key:value` on the line itself.

Mirrors CSS specificity: inline beats class, class beats type.

Complex values (`at:(x,y)`, `padding:(t,r,b,l)`) are replaced wholesale.

**Position conflicts:** `at:` always beats `cell:`.

---

## 14. SVG Output

### Document structure

```svg
<svg xmlns="http://www.w3.org/2000/svg"
     viewBox="X Y W H" width="W" height="H" class="plume">
  <style>
    @layer plume.defaults { :root, .plume { /* defaults */ } }
  </style>
  <defs>
    <!-- filters (shadow), clipPaths (fit=clip), symbols (icons) -->
  </defs>
  <g class="plume-scene"> <!-- scene tree --> </g>
  <g class="plume-wires"> <!-- wires --> </g>
</svg>
```

`viewBox` auto-sizes to content + `var(--plume-canvas-pad, 20)`.

### Node rendering

```svg
<g class="plume-node plume-shape-{type} plume-shape-{parent-type} plume-style-{s}"
   data-id="ID" transform="translate(X,Y)">
  <!-- shape geometry, then children -->
</g>
```

Auto-classes:
- `plume-node` — every scene node.
- `plume-shape-{name}` — the type plus every type it inherits from.
- `plume-style-{name}` — one per applied `.style`.

If `rotation:N`, transform becomes `translate(X,Y) rotate(N)`.

### Wire rendering

```svg
<g class="plume-wire plume-style-{s}" data-from="A" data-to="B">
  <path d="..." stroke="..." fill="none"/>
  <polygon class="plume-marker plume-marker-arrow" .../>
  <!-- text children at mid/start/end -->
</g>
```

### Standalone vs preprocessor mode

Standalone embeds the full `@layer plume.defaults` block. `--no-defaults` omits it (the host page is expected to supply the variables).

---

## 15. CLI

```
plume [options] <input.plume>
plume fmt [--check] [--stdout] <input.plume>
```

### Compile

| Flag | Meaning |
|---|---|
| `-o FILE` | Output path (default stdout). |
| `--format svg\|html` | `svg` (default) or HTML wrapper. |
| `--standalone` | Force embed of default CSS. |
| `--no-defaults` | Omit default CSS — host page supplies. |
| `--check` | Parse and validate only. |
| `--theme FILE` | CSS file with `--plume-*` overrides. |
| `--no-warn` | Suppress lint warnings. |
| `--strict` | Treat lint warnings as errors. |
| `--bake-vars` | Inline `var()` references as their resolved literals. |
| `-h`, `-V` | Help / version. |

`plume -` reads from stdin (filename `<stdin>` in errors).

### Format

`plume fmt` reformats a source file to the canonical style: 2-space indent, column-aligned id / type / label / attrs within a block, blank lines and comments preserved.

| Flag | Meaning |
|---|---|
| `--check` | Exit 1 if the file would be changed, but write nothing. |
| `--stdout` | Write formatted output to stdout instead of rewriting. |

`plume fmt -` reads stdin → stdout.

Exit codes: 0 success, 1 parse/resolution error or `--check` reformat needed, 2 I/O, 3 invalid CLI.

---

## 16. Errors

Format: `filename:line:col: error: <message>` (LSP-compatible).

| Condition | Message |
|---|---|
| Duplicate id | `duplicate id 'X' (previously at L:C)` |
| Ambiguous wire endpoint | `endpoint 'X' is ambiguous; qualify with full path` |
| Wire references unknown id / path | `wire endpoint 'X' not found` |
| Wire chain mixes operators | `wire chain mixes operators 'X' and 'Y'` |
| Wire chain < 2 nodes | `wire requires at least two endpoints` |
| Unknown type / style | `unknown type '\|X\|'` / `unknown style '.X'` |
| Inheritance cycle / depth > 16 | `cycle in 'X' → … → 'X'` / `'X' exceeds max inheritance depth (16)` |
| Forward reference | `'X' used before its definition (L:C)` |
| Defs block not first | `defs block must be the first statement` |
| Missing required attr | `'\|line\|' requires 'points'` |
| Unknown attr | `unknown attr 'foo' on '\|rect\|'` (warning) |
| Wire body non-text | `wire body may only contain \|text\| primitives` |
| Wire text uses node anchor | `wire labels accept only start/mid/end/0..1` |
| Invalid color / out-of-range | `invalid color 'XYZ'` / `rgb(300, 0, 0): component out of range` |
| Reserved identifier | `'rect' is reserved` |
| Grid placement out of range | `cell:(5, _) exceeds grid cols=3` |
| `\|slant\| skew` out of range | `skew:N must be in (-89, 89)` |
| Unknown icon name | `unknown icon name 'XYZ'` |
| `col-widths` / `row-heights` length mismatch | `col-widths has N values but grid cols=M` |
| Whitespace around `:` | `binding ':' must have no whitespace on either side` |
| Visual attr inline (lint) | `visual attr 'fill' inline; consider moving to a style` (warning) |

### Visual attrs (lint warning category)

`fill`, `stroke`, `thickness`, `stroke-style`, `opacity`, `radius`, `double`, `rotation`, `shadow`, `weight`, `align`, `fit`, `variant`, and `size` when applied to a `|text|` node.

Structural and always inline-OK: type / class / id / label / href / `title` / `aria-label`, placement (`at`, `offset`, `cell`, `span`, `z`), container (`layout`, `gap`, `padding`, `col-widths`, `row-heights`), geometry (`size`, `points`, `d`, `skew`), wire `marker*`, and `size` / `name` on `|icon|`.

---

## 17. Formal Grammar (EBNF)

```
file           = [ defs_block ] { stmt | comment | newline } EOF
defs_block     = "{" { defs_line | comment | newline } "}"

defs_line      = scene_config | var_override | style_def | shape_def
scene_config   = "|scene|" { attr } newline_or_semi    # at most one per file
var_override   = "--" ident ":" value newline_or_semi
style_def      = "." ident { style_ref | attr } newline_or_semi
shape_def      = type_def_ref { style_ref | attr } [ "{" body "}" ] newline_or_semi
type_def_ref   = "|" ident ":" ident "|"          # new shape : base

stmt           = node_decl | wire_decl
node_decl      = ident [ type_use_ref ] [ string [ string ] ]
                 { style_ref | attr } [ "{" body "}" ] newline_or_semi
primitive_decl = type_use_ref [ string [ string ] ]
                 { style_ref | attr } [ "{" body "}" ] newline_or_semi
type_use_ref   = "|" ident "|"

body           = { node_decl | primitive_decl | wire_decl | comment | newline }

wire_decl      = endpoint_group wire_op endpoint_group { wire_op endpoint_group }
                 [ string ] { style_ref | attr }
                 [ "{" { text_decl } "}" ] newline_or_semi
endpoint_group = endpoint { "&" endpoint }
endpoint       = ident { "." ident } [ "." side ]
side           = "t" | "b" | "l" | "r" | "top" | "bottom" | "left" | "right"
text_decl      = "|text|" string [ string ] { attr } newline_or_semi

wire_op        = [ start_marker ] line [ end_marker ]
line           = "-" | "--" | "-.-" | "=" | "~"
start_marker   = "<" | ">" | "o" | "<>"
end_marker     = ">" | "<" | "o" | "<>"

attr           = ident ":" value                  # no whitespace around ":"
style_ref      = "." ident                        # whitespace before required

value          = number | string | color | tuple | list | ident | plume_var
tuple          = "(" value { "," value } ")"
list           = "[" [ value { "," value } ] "]"
color          = "#" hexdigit{3|6|8} | css_name
               | "rgb(" … ")" | "rgba(" … ")" | "hsl(" … ")" | "none"
plume_var      = "--" ident { "-" ident }

number         = [ "+" | "-" ] ( digit+ [ "." digit+ ] | "." digit+ )
string         = '"' { unicode-char | escape } '"'
escape         = "\\" ( '"' | "\\" | "n" | "t" )
ident          = ( letter | "_" ) { letter | digit | "_" | "-" }
comment        = "//" { not-newline } newline
newline_or_semi = newline | ";"
```

LL(1) — single-token lookahead suffices. A hand-written recursive-descent parser fits in ~600 LOC.

---

## 18. Implementer Algorithm

Reference pipeline. Implementations may differ provided observable output matches.

**Phase 1 — Parse.** Lex into tokens, then recursive-descent into an AST.

**Phase 2 — Resolve.** Walk top-to-bottom:

1. **Defs block:**
   - Merge built-in defaults ← `--theme FILE` ← `--name:value` lines.
   - Apply `|scene|` config to the root scene container. If absent, default to `layout:row gap:--gap padding:--canvas-pad`.
   - Register styles (resolve `.other` refs already in table; detect cycles).
   - Register shapes (resolve base; detect cycles + depth > 16).

2. **Scene tree:**
   - Walk node declarations, resolving each node's `|type|` and `.style`s; merge attrs per [§ 13](#13-specificity--application-order).
   - For each wire endpoint referenced but not declared, auto-create a root-level `|rect|` node with label = id.
   - Shape instances expand their definition's body, scoping internal IDs under the instance ID.

3. **Wires:**
   - Resolve endpoint paths via suffix-match against the scene tree.
   - Reject ambiguous matches; require fully-qualified paths.
   - Merge wire attrs per [§ 13](#13-specificity--application-order).

Forward references (other than wire-to-undeclared-id auto-creation) or unknown names → error per [§ 16](#16-errors).

**Phase 3 — Layout.** Compute bboxes bottom-up:

1. Leaf primitives: bbox from `size:` (or per-shape defaults), with stroke contribution (half `thickness` per side).
2. Containers: lay out children per `layout:row` / `layout:column` (1D flex) or `layout:(C, R)` (2D grid). Grid places by explicit `cell:(c, r)` or declaration order, respecting `span:(c, r)`.
3. `at:` children skip flow but expand parent bbox. `at:out-*` is computed against parent bbox-excluding-out-children.
4. Apply `padding` to the container bbox, then position the node in its parent (`at:`, `offset:`).
5. `rotation` applies last as an SVG transform.

**Phase 4 — Route wires.** For each wire:

1. Get source/target bboxes post-layout.
2. Pick entry/exit edges — explicit `.side` wins, else nearest edge (tie → right > bottom > left > top).
3. Compute orthogonal route via A* with bend penalty.
4. Self-loops emit a fixed-shape loop.
5. Place markers (sized `max(--arrow-head, thickness × 5)`) with tip 1 px from the endpoint.
6. Place wire-text children at requested anchors.

For fans (`a -> b & c`), generate one wire per cartesian pair.

**Phase 5 — Render.** Depth-first emit SVG per [§ 14](#14-svg-output).

---

## 19. Reserved Words

User identifiers cannot use:

- **Layout values:** `row`, `column`, `grid`.
- **Alignment values:** `start`, `center`, `end`, `stretch`, `between`, `around`, `evenly`.
- **Anchors (node):** `top`, `bottom`, `left`, `right`, the 4 corner names, and the 8 `out-*` variants.
- **Endpoint sides:** `t`, `b`, `l`, `r` (in addition to the four full names above).
- **Anchors (wire-route):** `mid` (`start`/`end` overlap with alignment values; context-resolved).
- **Origin values:** `top-left`.
- **Primitives:** `rect`, `oval`, `line`, `arrow`, `path`, `poly`, `text`, `hex`, `slant`, `cyl`, `diamond`, `cloud`, `icon`, `image`.
- **Templates:** `group`, `circle`, `badge`, `button`, `card`, `note`, `db`, `table`, `cell`, `dim`.
- **Special types:** `scene` (used as `|scene|` to configure the root container in the defs block).
- **Constants:** `true`, `false`, `none`, `auto`.
- **Functions:** `rgb`, `rgba`, `hsl`.

---

## 20. Non-Goals (v2)

Out of scope; v2 syntax remains forward-compatible.

- Auto-layout (graph routing, force-directed placement).
- Multi-file imports.
- Animation; interactivity beyond `href`.
- Programmatic API (DSL only).
- Manual wire waypoints; double-stroke wires.
- Wrapping layouts (`flow`, `wrap`).
- Unicode identifiers; block comments; line continuation.
- Partial wires (`a ->` or `-> a`).
- Per-edge padding/gap keys (`padding-top:`) — use the `(t,r,b,l)` tuple.
- Embedded local images (URLs only).
- Cross-instance addressing into a shape definition (e.g., wiring from outside *into* a specific internal node of another instance via a path the definition doesn't expose). Internal wires inside shape defs work; dot-path access from outside works; what's deferred is implicit re-entry from external wires modifying internal structure.

---

## 21. Complete Example

```
{
  |scene| layout:(3,2) gap:40 padding:20 background:--bg

  --gap:24
  --accent:#0a84ff

  .thin    stroke:#444 thickness:1
  .bold    weight:bold
  .power   stroke:red thickness:2
  .signal  stroke:blue stroke-style:dashed
  .ghost   opacity:0.3

  |psu:rect|     radius:5
  |bus:slant|    fill:gray
  |alert:circle| stroke:red size:36

  |force_diagram:group| layout:column gap:8 padding:8 {
    |rect|  size:(100, 40) fill:lightblue radius:4
    |line|  points:[(-50, 0), (50, 0)] .thin
    |arrow| points:[(0, 0), (50, 20)]
    |text|  "Cavity" at:top size:12
  }

  |microservice:group| layout:column gap:8 {
    api |rect| "API"
    db  |cyl|  "DB"
    api -> db "queries"
  }
}

outlet |oval| "Outlet 120-240 VAC" cell:(1,1)

rails |group| "Power Rails" cell:(2,1) layout:column gap:20 {
  rail48 |group| "48V Rail" layout:row gap:10 {
    drive |psu| "Drive PSU 960W"
    bus48 |bus| "Bus"
  }
  rail24 |group| "24V Rail" layout:row gap:10 {
    ctrl  |psu| "Control PSU 240W"
    bus24 |bus| "Bus"
  }
}

consumers |group| "Consumers" cell:(3,1) layout:column gap:20 {
  booster |group| "Booster" layout:row gap:15 {
    fuse |alert| "60A Fuse" { |badge| "HOT" }
    caps |rect|  "MOSFET + 20x Caps" double:4 size:(80, 40) fill:white
  }
  heaters |group| "Heaters" layout:row gap:15 {
    ssr   |rect| "3x SSR"          double:4 size:(60, 40)
    bands |rect| "6x Band Heaters" double:4 size:(80, 40)
  }
}

fadec |group| "FADEC" cell:(1,2) layout:column gap:10 {
  estop |icon| name:power_settings_new size:32
}

auth  |microservice| "Auth"  cell:(2,2)
users |microservice| "Users" cell:(3,2)

fd1 |force_diagram| at:(900, 700)

// wires
outlet.r -> drive.l -> bus48 -> fuse -> caps .power
outlet -> ctrl -> bus24
bus48 -> ssr -> bands
fadec <-> drive "CAN"
estop --o fuse .power

auth.db -> users.api "replicates"
```

### Quick snippets — table + dimension line

```
specs |table| layout:(3, 3) col-widths:[80, 140, 80] row-heights:28 {
  |cell| "Voltage" weight:bold; |cell| "Current" weight:bold; |cell| "Power" weight:bold
  |cell| "48 V";                |cell| "20 A";               |cell| "960 W"
  |cell| "24 V";                |cell| "10 A";               |cell| "240 W"
}

dim1 |dim| points:[(0, 200), (300, 200)] {
  |text| "300 mm" at:center fill:#666 size:11
}
```

### Mermaid-fast quick diagram

```
A -> B -> C            // 3 implicit rects, 2 wires
D & E -> F             // fan-in: D→F, E→F
G ~> H                 // wavy arrow
I =o J                 // double line, dot end
```
