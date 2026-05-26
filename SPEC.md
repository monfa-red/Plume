# Plume — Language Specification (v1)

A small, human-readable language for plain-text diagrams. Flex/grid layout when you want it, composable primitives when you don't, CSS-driven theming throughout. Compiles to clean SVG.

This document is complete — an implementer should be able to build a conforming engine from it alone.

---

## Table of Contents

1. [Mental Model](#1-mental-model)
2. [File Format & Lexical Syntax](#2-file-format--lexical-syntax)
3. [The Five Blocks](#3-the-five-blocks)
4. [Node Declarations](#4-node-declarations)
5. [Layout](#5-layout)
6. [Positioning & Anchors](#6-positioning--anchors)
7. [Built-in Primitives](#7-built-in-primitives)
8. [Built-in Templates](#8-built-in-templates)
9. [Wires](#9-wires)
10. [Attribute Reference](#10-attribute-reference)
11. [Variables & Defaults](#11-variables--defaults)
12. [Specificity / Application Order](#12-specificity--application-order)
13. [SVG Output](#13-svg-output)
14. [CLI](#14-cli)
15. [Errors](#15-errors)
16. [Formal Grammar (EBNF)](#16-formal-grammar-ebnf)
17. [Implementer Algorithm](#17-implementer-algorithm)
18. [Reserved Words](#18-reserved-words)
19. [Non-Goals (v1)](#19-non-goals-v1)
20. [Complete Example](#20-complete-example)

---

## 1. Mental Model

A Plume file has **five top-level blocks** in strict order. All are optional except `scene`:

```
defaults { ... }   // override built-in CSS variables
styles   { ... }   // attribute bundles, applied via .name
shapes   { ... }   // type definitions
scene    { ... }   // the diagram
wires    { ... }   // connections
```

**One pass, no forward references.** A style or shape must be defined above its first use.

**Two reference sigils:**

| Sigil | References | Example |
|---|---|---|
| `:name` | A type — built-in primitive, built-in template, or user shape. | `outlet :oval "Outlet"` |
| `.name` | A style from `styles {}`. | `drive :psu .bold` |

A sigil following an identifier requires whitespace before it (`outlet :oval`, not `outlet:oval`). Multiple style refs are space-separated: `drive :psu .bold .ghost` (left-to-right application).

**Identifiers** sit in a fixed positional slot (first token of a declaration), so they need no sigil.

**Nothing is hardcoded.** Every default the engine uses (colors, fonts, sizes, padding, gaps) is a named CSS variable. Built-in fallbacks ship with the engine; the `defaults {}` block, a `--theme` file, or user CSS can override any of them. See [§ 11](#11-variables--defaults).

---

## 2. File Format & Lexical Syntax

| Property | Value |
|---|---|
| Extension | `.plume` |
| Encoding | UTF-8 (BOM ignored) |
| Line endings | LF or CRLF (CRLF normalized on read) |
| Comments | `// ...` to end of line. No block comments. |
| Statement ends | newline or `;` (no line continuation) |
| Identifier | `[a-zA-Z_][a-zA-Z0-9_-]*` — case-sensitive, ASCII only |

Whitespace between tokens is insignificant except as a separator.

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

`#fff`, `#ffaa00`, `#ffaa00cc` (alpha), CSS named colors (`red`, `cornflowerblue`), `rgb(...)`, `rgba(...)`, `hsl(...)`, `var(name)`, or `none`. Out-of-range channel components are an error.

---

## 3. The Five Blocks

### 3.1 `defaults`

Overrides built-in CSS variables. Each line is `name=value`; each name becomes the CSS variable `--plume-name` in the emitted SVG.

```
defaults {
  gap=24
  padding=16
  fill=white
  font="Inter, system-ui, sans-serif"
  accent=#0a84ff
  origin=top-left          // flip default origin for the whole diagram
}
```

**Layered precedence — last wins:**
```
built-in fallback → --theme file → defaults {} block → runtime CSS
```

Layout-affecting variables (`gap`, `padding`, `rect-w`, …) bake at compile time, so runtime CSS cannot change layout. Visual variables (`fill`, `stroke`, `font`, colors) emit as live `var(--plume-*)` references and respond to runtime CSS. Full variable list in [§ 11.1](#111-built-in-css-variables).

### 3.2 `styles`

Reusable attribute bundles. Each style has no shape of its own; it's applied via `.name`.

```
styles {
  thin    stroke=#444 thickness=1
  bold    weight=bold
  power   stroke=red  thickness=2
  base    thickness=1 stroke=#444
  warn    .base stroke=orange       // composes another style
}
```

Application is left-to-right; later wins. Cycles are an error. Styles may be applied anywhere attrs are accepted.

### 3.3 `shapes`

User-defined types. **One unified form:**

```
name [:base] [attrs…] [{ body }]
```

At least one of `:base` or `{ body }` is required.

```
shapes {
  // attr bundle (no body)
  psu :rect radius=5
  bus :slant fill=gray

  // container with layout
  toolbar layout=row gap=10 {
    :icon name=save
    :icon name=copy
  }

  // base + body — full composition
  panel :group layout=column gap=8 padding=12 {
    :text "Title" at=top weight=bold
    :text "Content"
  }
}
```

The base may be any built-in primitive, built-in template, or previously-defined shape. Max inheritance depth: **16**. Coordinates inside the body are **local** (center-origin). Layout applies if `layout=` is set; otherwise children position absolutely. The shape becomes usable as `:name` like a built-in.

### 3.4 `scene`

Composes the diagram. The block opener accepts the same layout attrs as any container:

```
scene layout=grid cols=3 gap=40 padding=20 {
  outlet :oval "Outlet 120-240 VAC" col=1
  drive  :psu  "Drive PSU"          col=2 .bold
  bus48  :bus  "Bus"                col=3
}
```

### 3.5 `wires`

Connections between scene IDs. Block-level attrs apply to every wire inside; per-wire attrs override. Full syntax in [§ 9](#9-wires).

```
wires stroke=#444 thickness=1 {
  outlet -> drive -> bus48
  estop  --> fuse .power
}
```

---

## 4. Node Declarations

The form used in `scene`, in shape bodies, and in any node with children:

```
id :type "label?" .style…? attrs… { children? }
```

Order on the line: `id` → `:type` → `"label"` → styles/attrs (may interleave) → `{ body }`. Inside a shape body, the `id` slot is omitted — primitives are anonymous.

A newline or `;` ends a declaration.

### Bare attrs

A fixed set of attrs accept the bare form (the name alone, no `=value`), meaning "enabled with default":

| Attr | Bare meaning |
|---|---|
| `dashed` | `dashed=(4,4)` |
| `dotted` | `dotted=(1,3)` |
| `double` | `double=(4,-4)` |
| `shadow` | `shadow=(2,2,4,var(--plume-shadow))` |

All other attrs require `=value`.

### Label sugar

`id :type "label"` expands to a `:text` child:

```
drive :psu "Drive PSU 960W"
// equivalent to:
drive :psu { :text "Drive PSU 960W" }
```

If both a sugar label and explicit `:text` children are present, the sugar's text is added first.

Multi-line labels: use `\n` (`"Drive PSU\n960W"`). The text bbox sizes to the widest line; vertical spacing is `size × 1.2`.

Wires support the same sugar — `a -> b "label"` adds a `:text "label" at=mid` child (see [§ 9](#9-wires)).

---

## 5. Layout

Any container picks a layout:

| `layout=` | Behavior |
|---|---|
| `column` | Vertical flex (default for containers with children) |
| `row` | Horizontal flex |
| `grid` | 2D grid — requires `cols=N` or `rows=N` |

### Container attrs

| Attr | Applies to | Notes |
|---|---|---|
| `gap` | all | Spacing between children. Scalar, `(y,x)`, or `(t,r,b,l)`. Negative allowed (overlap). |
| `padding` | all | Inner padding. Same value forms as `gap`. |
| `cols`, `rows` | grid | Track count. At least one required. |
| `col-widths`, `row-heights` | grid | Fixed track sizes. Scalar = all equal; list = explicit per track. List length must equal `cols`/`rows`. |
| `h`, `v` | all | Axis alignment / distribution (see below). |
| `background` | scene only | Canvas background color. |

When `col-widths` / `row-heights` are set, cells take exactly those sizes (children with explicit `w`/`h` still override). When omitted, the grid auto-sizes cells to the widest/tallest child in each track.

### Multi-value `padding`, `gap`, `radius`

| Form | Meaning |
|---|---|
| `N` | All four sides |
| `(y, x)` | Vertical, horizontal |
| `(t, r, b, l)` | Clockwise from top |

For `radius`: 2-val = `(top-corners, bottom-corners)`.

### `h=` and `v=` values

The same value names work for both axes; the layout type determines which is the stacking (main) axis.

| Value | Stacking axis | Cross axis |
|---|---|---|
| `start`, `center`, `end` | Pack at edge / centered / opposite edge | Align child to edge / center / opposite |
| `stretch` | (no effect) | Children fill the cross axis |
| `between`, `around`, `evenly` | Distribute with equal gaps | (no effect — treated as `start`) |

For `row`, stacking axis is horizontal (`h=`); for `column`, vertical (`v=`); for `grid`, both axes are stacking and `h`/`v` align cell content.

### Child positioning

| Attr | Effect |
|---|---|
| `at=(x,y)` | Place child's center at (x,y). Removes from flow. |
| `at=anchor` | Named anchor — see [§ 6](#6-positioning--anchors). |
| `offset=(x,y)` | Fine-tune from an anchor. |
| `col=N`, `row=N` | Grid track (1-indexed). |
| `colspan=N`, `rowspan=N` | Span multiple tracks (default 1). |
| `z=N` | Render-order override. Source order is the tiebreak. |

`at=` always beats `col`/`row`. The absolutely-positioned child still contributes to the parent's bbox. Out-of-range `col`/`row` is an error.

---

## 6. Positioning & Anchors

The **bounding box (bbox)** of a shape is the smallest axis-aligned rectangle that fully contains it, including its stroke.

### Positioning rules

1. **Center origin.** Every shape's bbox is centered at the parent's coordinate origin by default. `at=(x,y)` puts the bbox center at (x,y). This differs from CSS `position: absolute` (which is top-left); the convention is chosen because diagram authors think in centers.
2. **`origin=top-left`** opts into CSS-style top-left positioning per instance (or globally via `defaults`).
3. **Source order = render order.** Later renders on top. `z=N` overrides; ties broken by source order.
4. **Strokes count toward bbox** — `:rect w=100 h=50 thickness=4` has bbox 104×54.
5. **`:path` is the only exception to center-origin** — `d=` uses native SVG top-left coordinates.
6. **Rotation** applies last as an SVG transform; the rotated bounding rectangle is what propagates up the tree.

### Anchors

Anchors are bare names that resolve to positions relative to the parent's bbox.

**Inside the parent:** `center`, `top`, `bottom`, `left`, `right`, `top-left`, `top-right`, `bottom-left`, `bottom-right`.

**Outside the parent** (places the child's facing edge tangent to the parent's): `out-top`, `out-bottom`, `out-left`, `out-right`, plus the four corner variants `out-top-left`, etc.

`offset=(x,y)` shifts from any anchor. **Out-* anchors are computed against the parent's bbox excluding out-* children** — preventing infinite recursion (child outside → grows parent → moves anchor → …).

**Wire-route anchors** (only valid on a `:text` child of a wire): `start`, `mid`, `end`, or a fractional number `0..1` along the route.

### Auto-sizing

Closed shapes auto-size to their text children + `var(--plume-text-pad, 16)` on each side when dimension attrs are omitted. Text bbox width comes from embedded font metrics (reproducible across hosts; approximate for non-default fonts).

If neither dimensions nor text are given, defaults apply (from CSS vars; fallback values shown):

| Shape | Default |
|---|---|
| `:rect`, `:group`, `:slant` | `w=100 h=40` |
| `:oval` | `rx=30 ry=20` |
| `:hex`, `:cyl`, `:diamond`, `:cloud` | `w=60 h=60` |
| `:icon` | `24 × 24` |
| `:poly`, `:image` | Error if required attrs missing |

`:line` and `:arrow` always require explicit `from` and `to`.

---

## 7. Built-in Primitives

14 primitives total. All accept position attrs and visual style attrs; closed shapes also accept `double`, `rotation`, `shadow`.

| Primitive | Required | Notes |
|---|---|---|
| `:rect` | `w h` (auto) | Rounded corners via `radius=` (scalar / 2-val / 4-val per [§ 5](#5-layout)). |
| `:oval` | `rx ry` (auto) | A circle is `rx == ry`; use the `:circle` template ([§ 8](#8-built-in-templates)) for sugar. |
| `:hex` | `w h` (auto) | Regular hex, flat top/bottom. Uses shorter dimension if ratio ≠ 2:√3. |
| `:slant` | `w h` (auto) | Parallelogram, top edge shifted by `tan(skew) × h`. `skew` in degrees, range (-89, 89). |
| `:cyl` | `w h` (auto) | Cylinder (database icon). `h` is body height; top/bottom ellipses extend ±h/8. |
| `:diamond` | `w h` (auto) | Rhombus inscribed in w×h. |
| `:cloud` | `w h` (auto) | Stylized cloud, fixed path template scaled to fit. |
| `:poly` | `points=[(x,y),…]` | ≥3 points. Local coords (center-origin). |
| `:path` | `d="..."` | Raw SVG path. **Native top-left coords** (only exception). |
| `:text` | label string | See [§ 4 label sugar](#label-sugar) and [§ 10 text attrs](#text). |
| `:line` | `from to` | Markers via `marker*=` attrs. |
| `:arrow` | `from to` | A `:line` with `marker-end=arrow` by default. |
| `:icon` | `name` | Material Symbols. `variant=outlined\|filled\|rounded\|sharp`, `size=N`. Compiler bundles only referenced icons. |
| `:image` | `href w h` | Emits `<image href="...">`. External URLs only; no embedding. |

### Visual modifiers

These attrs apply to closed shapes (where meaningful):

| Attr | Forms | Effect |
|---|---|---|
| `double` | bare / `N` / `(x,y)` | Render twice with offset, second copy behind. Default offset `(4, -4)`. |
| `rotation` | `N` degrees | Rotate around bbox center. Emitted as `transform="rotate(...)"`. |
| `shadow` | bare / `N` / `(dx,dy)` / `(dx,dy,blur)` / `(dx,dy,blur,color)` | Drop shadow via SVG `<filter>`. Bare defaults `(2,2,4,var(--plume-shadow))`. |

### Markers (on `:line`, `:arrow`, and wires)

Three attrs control endpoint markers:

| Attr | Effect |
|---|---|
| `marker=X` | Shorthand: both ends. |
| `marker-start=X` | Start end (or wire source). |
| `marker-end=X` | End end (or wire target). |

Marker values: `none`, `arrow` (scales with `thickness`; size = `max(var(--plume-arrow-head, 10), thickness × 5)`), `dot` (filled circle), `diamond` (filled rhombus), `crow` (crow's-foot).

**Per-type defaults:**

| Type | start | end |
|---|---|---|
| `:line` | none | none |
| `:arrow` | none | arrow |
| Wire `->` `-->` `-.->` | none | arrow |
| Wire `<-` `<--` `<-.-` | arrow | none |
| Wire `<->` `<-->` `<-.->` | arrow | arrow |

Source-order wins on conflicts: `marker=arrow marker-end=dot` → start=arrow, end=dot. Reverse the order and both ends are arrows (because `marker=` is later and replaces both).

Marker color = stroke color.

---

## 8. Built-in Templates

Each template is an attribute bundle over a primitive base. Equivalent to a user-defined shape — just shipped with the engine.

| Template | Base | Defaults | Use for |
|---|---|---|---|
| `:group` | `:rect` | `dashed stroke=muted fill=none padding=15`; text `at=top weight=bold` | Frame + label slot for grouping. |
| `:circle` | `:oval` | `r=N` sugar → `rx=N ry=N`. Default `r=20`. | Convenience for circles. |
| `:badge` | `:rect` | `at=top-right radius=999 padding=(2,8) shadow fill=accent z=10`; text small + on-accent | Floating pill on a parent's corner. |
| `:button` | `:rect` | `radius=4 padding=(8,16) shadow fill=accent`; text on-accent | Pair with `href=` to actually click. |
| `:card` | `:rect` | `radius=8 padding=16 shadow stroke=none fill=fill` | Content surface, no border. |
| `:note` | `:rect` | `radius=2 padding=12 shadow stroke=none fill=note-bg` | Sticky-note look. |
| `:db` | `:cyl` | (alias) | Database, friendlier name. |
| `:table` | `:group` | `layout=grid gap=0 padding=0 stroke=none` | Container for `:cell`s; use with `cols=`, `col-widths=`, `row-heights=`. |
| `:cell` | `:rect` | `padding=8 stroke=stroke thickness=1 fill=none` | Bordered cell. Sizes to its grid slot. |
| `:dim` | `:line` | `marker=arrow` (both ends) | Dimension line. Add a `:text at=center` child for the label. |

Defaults that reference `--plume-*` vars (e.g. `accent`, `fill`, `note-bg`) resolve via the variable system in [§ 11](#11-variables--defaults). Templates can be extended in `shapes {}` like any user shape.

---

## 9. Wires

Wires connect scene-node IDs.

### Operators

| Op | Style | Direction |
|---|---|---|
| `->` `<-` `<->` | solid | forward / reverse / bidirectional |
| `-->` `<--` `<-->` | dashed | (same) |
| `-.->` `<-.-` `<-.->` | dotted | (same) |

A **chain** repeats a single operator: `a -> b -> c -> d`. Mixing operators within one chain is a parse error.

### Wire syntax

```
id1[anchor?] OP id2[anchor?] [OP id3[anchor?] …] "label?" .style…? attrs… { children? }
```

A chain requires at least two nodes. Children may only be `:text` (wire labels). Block-level attrs on `wires { ... }` are defaults for each wire inside.

### Endpoint anchors

Append `[anchor]` to any node ID to force the edge the route uses at that endpoint. Anchors: `top`, `bottom`, `left`, `right`, `top-left`, `top-right`, `bottom-left`, `bottom-right`. No whitespace before `[`.

```
outlet[right] -> drive[left] -> bus48[left]
fadec[right]  <-> drive[left] "CAN"
```

If omitted, the engine picks the edge nearest the other node (tie → right > bottom > left > top).

### Label sugar

`a -> b "label"` expands to `a -> b { :text "label" at=mid }`. For chains, the label sits at the midpoint of the **overall route**.

### Wire-text children

```
a -> b {
  :text "label" at=mid size=10
  :text "↓"     at=0.75
}
```

`at=` accepts wire-route anchors only (`start`, `mid`, `end`, or `0..1`). `offset=(x,y)` shifts in the route's local tangent frame: `x` along the route, `y` perpendicular.

### Markers

Wires use the same `marker=` / `marker-start=` / `marker-end=` attrs as `:line` and `:arrow`. The operator sets the defaults (see [§ 7 Markers](#markers-on-line-arrow-and-wires)); explicit attrs override.

```
a -> b marker=dot              // dots at BOTH ends (overrides default arrow at b)
a -> b marker-end=dot          // dot at b only (start stays at default 'none')
a <-> b marker-start=crow      // crow at a, arrow at b
```

### Routing

Orthogonal L- or Z-bend between source and target bboxes. The engine picks L vs Z by relative position. Markers are inset 4 px from their endpoint.

**Self-loops** (`a -> a`): a small loop exits the right edge, curves over the top, re-enters the top edge (diameter = `var(--plume-rect-h, 40) × 0.75`).

**Duplicate wires** between the same pair are allowed — they render as separate paths.

Manual waypoints are not in v1.

---

## 10. Attribute Reference

Comprehensive list; see linked sections for context.

### Visual

| Attr | Type | Default |
|---|---|---|
| `fill` | color | `var(--plume-fill)` for closed shapes; `var(--plume-text-color)` for text; `var(--plume-stroke)` for icons |
| `stroke` | color | `var(--plume-stroke)`. On `:line`/`:arrow`, the line color. |
| `thickness` | number | `var(--plume-thickness, 1)`. Canonical; `stroke-width` not accepted. |
| `dashed` | bare / N / (N,M) | off |
| `dotted` | bare / N | off |
| `opacity` | 0..1 | 1 |
| `radius` | scalar / (y,x) / (t,r,b,l) | `var(--plume-radius, 0)`. `:rect` only. |
| `double`, `rotation`, `shadow` | see [§ 7](#visual-modifiers) | off / 0 / off |
| `marker`, `marker-start`, `marker-end` | see [§ 7](#markers-on-line-arrow-and-wires) | per-type |

### Geometry

| Attr | Type | Notes |
|---|---|---|
| `at` | `(x,y)` or anchor | `(x,y)` = bbox center at (x,y) (overridable via `origin=top-left`). |
| `offset` | `(x,y)` | From anchor. No effect on `at=(x,y)`. |
| `w`, `h`, `r`, `rx`, `ry` | number | Dimensions. |
| `from`, `to` | `(x,y)` | Line/arrow endpoints. |
| `points` | `[(x,y), …]` | Polygon vertices (≥3). |
| `d` | string | Raw SVG path data. |
| `skew` | number | Slant, degrees. |
| `origin` | `center` / `top-left` | Bbox origin reference. |
| `z` | integer | Render-order override. |

### Container & grid

`layout`, `gap`, `padding`, `cols`, `rows`, `col-widths`, `row-heights`, `colspan`, `rowspan`, `h`, `v`, `background` — see [§ 5](#5-layout).

### Text

| Attr | Default | Notes |
|---|---|---|
| `at` | `center` | Anchor or `(x,y)`. |
| `align` | `center` | `left`/`center`/`right` — controls multi-line alignment within text bbox. |
| `size` | `var(--plume-text-size, 13)` | Font size, px. |
| `weight` | `normal` | `normal` / `bold`. |
| `fill` | `var(--plume-text-color)` | Text color. |
| `font` | `var(--plume-font)` | Falls through to browser default if unset. |
| `fit` | `none` | `none` / `shrink` / `wrap` / `clip` — overflow behavior. |

`fit=shrink` uses SVG `textLength` + `lengthAdjust="spacingAndGlyphs"`. `fit=wrap` breaks on word boundaries into `<tspan>` lines. `fit=clip` uses `<clipPath>` on the container bbox.

### Accessibility & interaction

| Attr | Notes |
|---|---|
| `title` | Wraps the shape in `<title>` — browser tooltip + screen reader. |
| `aria-label` | Emitted on the `<g>`. |
| `href` | Wraps the shape (or wire) in `<a href>`. Whole shape becomes clickable. |

---

## 11. Variables & Defaults

All defaults live in CSS variables. Override at any level:

```
built-in fallback → --theme file → defaults {} block → runtime CSS (visual only)
```

**Layout variables** (gap, padding, dimensions) bake at compile time. **Visual variables** (colors, fonts) emit as live `var(--plume-*)` references and respond to runtime CSS.

### 11.1 Built-in CSS variables

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
  --plume-shadow        rgba(0,0,0,0.2)

Layout (compile-time):
  --plume-text-size     13
  --plume-text-pad      16
  --plume-gap           20
  --plume-padding       0
  --plume-thickness     1
  --plume-radius        0
  --plume-rect-w        100
  --plume-rect-h        40
  --plume-oval-rx       30
  --plume-oval-ry       20
  --plume-circle-r      20
  --plume-arrow-head    10
  --plume-icon-size     24
  --plume-canvas-pad    20
```

### 11.2 `var()` in attribute values

| Form | Emits | Use for |
|---|---|---|
| `var(name)` | `var(--plume-name)` | Plume defaults / your own `--plume-*` vars. The compiler prepends `--plume-`. |
| `var(--name)` | `var(--name)` | Raw passthrough — non-plume CSS vars (your app's design tokens). The leading `--` signals "don't prepend." |

Raw-passthrough vars have no compile-time value, so they can only be used for visual attrs (not layout).

### 11.3 `@layer plume.defaults`

In standalone mode the embedded `<style>` wraps default variables in `@layer plume.defaults { ... }`. Any unlayered host CSS automatically wins, no `!important` needed:

```css
.plume { --plume-accent: hotpink; }
[data-theme="dark"] .plume { --plume-bg: #111; --plume-fg: #eee; }
```

---

## 12. Specificity / Application Order

For any node, wire, or primitive, attrs merge in this order — **later wins**:

1. **Type defaults** (and parent types, recursively).
2. **Block-level defaults** (attrs on the enclosing block opener).
3. **Style classes** — applied left-to-right.
4. **Inline attrs** — `key=value` on the line itself.

Mirrors CSS specificity: inline beats class, class beats type.

Complex values (`at=(x,y)`, `padding=(t,r,b,l)`) are replaced wholesale — no per-component merging.

**Position conflicts:** `at=` always beats `col`/`row` (child positions absolutely).

**Multi-value attr conflicts:** when a shorthand and its component variants appear, source order wins — the later declaration replaces entirely.

---

## 13. SVG Output

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
<g class="plume-node plume-shape-{type} plume-shape-{parent-type} plume-style-{s1}"
   data-id="ID" transform="translate(X,Y)">
  <!-- shape geometry, then children -->
</g>
```

Auto-classes:
- `plume-node` — every scene node.
- `plume-shape-{name}` — the type plus every type it inherits from. A `:psu` based on `:rect` emits `plume-shape-psu plume-shape-rect`. CSS can target the specific shape or its base.
- `plume-style-{name}` — one per applied `.style`, in declaration order.

If `rotation=N`, transform becomes `translate(X,Y) rotate(N)`.

Example: `drive :psu "Drive" .bold .ghost` →

```svg
<g class="plume-node plume-shape-psu plume-shape-rect plume-style-bold plume-style-ghost"
   data-id="drive" transform="translate(...)"> ... </g>
```

Selectors that just work in your CSS:

```css
.plume-shape-psu { fill: navy }
.plume-style-bold text { font-weight: 700 }
```

### Wire rendering

```svg
<g class="plume-wire plume-style-{s}" data-from="A" data-to="B">
  <path d="..." stroke="..." fill="none"/>
  <polygon class="plume-marker plume-marker-arrow" .../>
  <!-- text children at mid/start/end -->
</g>
```

Markers carry `plume-marker plume-marker-{type}` (`-dot`, `-crow`, etc.).

### Standalone vs preprocessor mode

Standalone embeds the full `@layer plume.defaults` block. `--no-defaults` omits it (the host page is expected to supply the variables).

---

## 14. CLI

```
plume [options] <input.plume>
```

| Flag | Meaning |
|---|---|
| `-o FILE` | Output path (default stdout). |
| `--format svg\|html` | `svg` (default) or HTML wrapper. |
| `--standalone` | Force embed of default CSS (default outside preprocessor mode). |
| `--no-defaults` | Omit default CSS — host page supplies. |
| `--check` | Parse and validate only. |
| `--theme FILE` | CSS file with `--plume-*` overrides. Used for compile-time layout vars AND inlined into the SVG. |
| `-h`, `-V` | Help / version. |

`plume -` reads from stdin (filename `<stdin>` in errors).

Exit codes: 0 success, 1 parse/resolution error, 2 I/O, 3 invalid CLI.

---

## 15. Errors

Format: `filename:line:col: error: <message>` (LSP-compatible). Filename is `<stdin>` when reading stdin.

| Condition | Message |
|---|---|
| Duplicate scene ID | `duplicate scene id 'X' (previously at L:C)` |
| Wire references unknown ID | `wire references undefined id 'X'` |
| Wire chain mixes operators | `wire chain mixes operators 'X' and 'Y'` |
| Wire chain < 2 nodes | `wire requires at least two endpoints` |
| Unknown type / style | `unknown type ':X'` / `unknown style '.X'` |
| Sigil mismatch | `'X' is a style, not a type — use .X` (and inverse) |
| Inheritance cycle / depth > 16 | `cycle in 'X' → ... → 'X'` / `'X' exceeds max inheritance depth (16)` |
| Block out of order | `'shapes' must appear before 'scene'` |
| Forward reference | `'X' used before its definition (L:C)` |
| Missing required attr | `':rect' requires 'w' and 'h'` |
| Unknown attr | `unknown attr 'foo' on ':rect'` (warning) |
| Wire body non-`:text` | `wire body may only contain :text primitives` |
| Wire `:text` uses node anchor | `wire labels accept only start/mid/end/0..1` |
| Node `:text` uses wire anchor | `:text anchor 'mid' is wire-only; use 'top'/'center'/etc.` |
| Invalid color / out-of-range component | `invalid color 'XYZ'` / `rgb(300,0,0): component out of range` |
| Reserved identifier | `'styles' is reserved` |
| Grid placement out of range | `col=5 exceeds cols=3` |
| `:slant skew` out of range | `skew=N must be in (-89, 89)` |
| Unknown icon name | `unknown icon name 'XYZ' (not in Material Symbols)` |
| Invalid wire endpoint anchor | `wire endpoint anchor 'X' must be top/bottom/left/right or a corner` |
| `col-widths`/`row-heights` length mismatch | `col-widths has N values but cols=M` |

Implementations may add additional warnings.

---

## 16. Formal Grammar (EBNF)

```
file           = { comment | newline } { block } EOF

block          = "defaults" "{" defaults_body "}"
               | "styles"   "{" body "}"
               | "shapes"   "{" body "}"
               | "scene"    attrs "{" body "}"
               | "wires"    attrs "{" body "}"

defaults_body  = { default_decl | comment | newline }
default_decl   = ident "=" value newline_or_semi

body           = { statement | comment | newline }
statement      = style_def | shape_def | node_decl | wire_decl | primitive_decl

style_def      = ident { style_ref | attr } newline_or_semi
shape_def      = ident [ type_ref ] { style_ref | attr } [ "{" body "}" ] newline_or_semi
                 # at least one of type_ref or body required
node_decl      = ident type_ref [ string ] { style_ref | attr } [ "{" body "}" ] newline_or_semi
primitive_decl = type_ref [ string ] { style_ref | attr } [ "{" body "}" ] newline_or_semi

wire_decl      = wire_endpoint wire_op wire_endpoint { wire_op wire_endpoint }
                 [ string ]                          # label sugar
                 { style_ref | attr }
                 [ "{" { text_primitive_decl } "}" ]
                 newline_or_semi
wire_endpoint  = ident [ "[" anchor_name "]" ]
text_primitive_decl = ":text" string { attr } newline_or_semi

type_ref       = ":" ident
style_ref      = "." ident
attr           = ident "=" value | ident             # bare attr
attrs          = { attr }

value          = number | string | color | tuple | list | ident | var_ref
tuple          = "(" value { "," value } ")"         # 2..5 components
list           = "[" [ value { "," value } ] "]"
color          = "#" hexdigit{3|6|8} | css_name
               | "rgb(" ... ")" | "rgba(" ... ")" | "hsl(" ... ")" | "none"
var_ref        = "var" "(" ( ident | "--" ident { "-" ident } ) ")"

number         = [ "+" | "-" ] ( digit+ [ "." digit+ ] | "." digit+ )
string         = '"' { unicode-char | escape } '"'
escape         = "\\" ( '"' | "\\" | "n" | "t" )
ident          = ( letter | "_" ) { letter | digit | "_" | "-" }
anchor_name    = "top" | "bottom" | "left" | "right"
               | "top-left" | "top-right" | "bottom-left" | "bottom-right"
wire_op        = "->" | "<-" | "<->" | "-->" | "<--" | "<-->"
               | "-.->" | "<-.-" | "<-.->"
comment        = "//" { not-newline } newline
newline_or_semi = newline | ";"
```

LL(1) — single-token lookahead suffices throughout. A hand-written recursive-descent parser fits in ~600 LOC.

---

## 17. Implementer Algorithm

Reference pipeline. Implementations may differ provided observable output matches.

**Phase 1 — Parse.** Lex into tokens, then recursive-descent per [§ 16](#16-formal-grammar-ebnf) into an AST.

**Phase 2 — Resolve.** Walk top-to-bottom and build symbol tables in block order:

1. **Defaults** — merge built-in fallbacks ← `--theme FILE` ← `defaults {}` entries.
2. **Styles** — resolve each style's attrs (applying any `.other` refs already in the table).
3. **Shapes** — resolve `:base` and attrs; detect cycles and depth > 16.
4. **Scene** — resolve `:type` and `.style`s for each node; merge attrs per [§ 12](#12-specificity--application-order).
5. **Wires** — resolve referenced IDs; merge attrs.

Forward references or unknown names → error per [§ 15](#15-errors).

**Phase 3 — Layout.** Compute bboxes bottom-up:

1. Leaf primitives: bbox from explicit dims, with stroke contribution (half `thickness` per side).
2. Containers: lay out children per `column`/`row`/`grid`. Grid places by explicit `col`/`row` or declaration order, respecting `colspan`/`rowspan`; cells size by `col-widths`/`row-heights` if set, else auto-size.
3. `at=` children skip flow but still expand parent bbox. `at=out-*` is computed against parent bbox-excluding-out-children.
4. Apply `padding` to the container bbox, then position the node in its parent (`at=`, `offset=`).
5. `rotation` applies last as an SVG transform; the rotated bounding rectangle is what propagates up.

**Phase 4 — Route wires.** For each wire:

1. Get source/target bboxes post-layout.
2. Pick entry/exit edges — bracketed anchor wins, else nearest edge (tie → right > bottom > left > top).
3. Compute L- or Z-bend route.
4. Self-loops emit a fixed-shape loop.
5. Place markers (`arrow` / `dot` / `diamond` / `crow`, sized per `var(--plume-arrow-head)` or `thickness × 5`) inset 4 px from the endpoint.
6. Place wire-text children at requested anchors along the route.

**Phase 5 — Render.** Depth-first emit SVG per [§ 13](#13-svg-output).

---

## 18. Reserved Words

User identifiers cannot use:

- **Blocks:** `defaults`, `styles`, `shapes`, `scene`, `wires`.
- **Layout values:** `row`, `column`, `grid`, `start`, `center`, `end`, `stretch`, `between`, `around`, `evenly`.
- **Anchors (node):** `top`, `bottom`, `left`, `right`, the 4 corner names, and the 8 `out-*` variants.
- **Anchors (wire-route):** `mid` (`start`/`end` overlap with layout values; context-resolved).
- **Origin values:** `top-left`.
- **Primitives:** `rect`, `oval`, `line`, `arrow`, `path`, `poly`, `text`, `hex`, `slant`, `cyl`, `diamond`, `cloud`, `icon`, `image`.
- **Templates:** `group`, `circle`, `badge`, `button`, `card`, `note`, `db`, `table`, `cell`, `dim`.
- **Constants:** `true`, `false`, `none`, `auto`.
- **Functions:** `var`, `rgb`, `rgba`, `hsl`.

---

## 19. Non-Goals (v1)

Out of scope; v1 syntax remains forward-compatible.

- Auto-layout (graph routing, force-directed placement).
- Multi-file imports.
- Animation; interactivity beyond `href`.
- Programmatic API (DSL only).
- Manual wire waypoints; double-stroke wires.
- Wrapping layouts (`flow`, `wrap`).
- Unicode identifiers; block comments; line continuation.
- Partial wires (`a ->` or `-> a`).
- Per-edge padding/gap keys (`padding-top=`, etc.) — use the `(t,r,b,l)` tuple.
- Embedded local images (URLs only).

---

## 20. Complete Example

```
defaults {
  gap=24
  accent=#0a84ff
}

styles {
  thin   stroke=#444 thickness=1
  bold   weight=bold
  power  stroke=red  thickness=2
  signal stroke=blue dashed
  ghost  opacity=0.3
}

shapes {
  psu   :rect radius=5
  bus   :slant fill=gray
  alert :circle stroke=red r=18

  force_diagram {
    :rect  w=100 h=40 fill=lightblue radius=4
    :line  from=(-50,0) to=(50,0) .thin
    :arrow from=(0,0)  to=(50,20)
    :text  "Cavity" at=top size=12
  }
}

scene layout=grid cols=3 gap=40 padding=20 background=var(bg) {
  outlet :oval "Outlet 120-240 VAC" col=1 row=1 v=center

  rails :group "Power Rails" col=2 layout=column gap=20 {
    rail48 :group "48V Rail" layout=row gap=10 {
      drive :psu "Drive PSU 960W"
      bus48 :bus "Bus"
    }
    rail24 :group "24V Rail" layout=row gap=10 {
      ctrl  :psu "Control PSU 240W"
      bus24 :bus "Bus"
    }
  }

  consumers :group "Consumers" col=3 layout=column gap=20 {
    booster :group "Booster" layout=row gap=15 {
      fuse :alert "60A Fuse" { :badge "HOT" }
      caps :rect  "MOSFET + 20× Caps" double w=80 h=40 fill=white
    }
    heaters :group "Heaters" layout=row gap=15 {
      ssr   :rect "3× SSR"          double w=60 h=40
      bands :rect "6× Band Heaters" double w=80 h=40
    }
  }

  fadec :group "FADEC" col=1 row=2 layout=column gap=10 {
    estop :icon name=power_settings_new size=32
  }

  fd1 :force_diagram at=(900, 700)
}

wires stroke=var(stroke) thickness=1 {
  outlet -> drive -> bus48 -> fuse -> caps .power
  outlet -> ctrl  -> bus24
  bus48  -> ssr   -> bands

  // brackets where they actually matter: force a specific edge instead of nearest-edge auto-pick
  fadec[right] <-> drive[left] "CAN"

  estop --> fuse .power stroke=orange marker-end=dot
}
```

### Quick snippets — table + dimension line

```
specs :table cols=3 col-widths=[80,140,80] row-heights=28 {
  :cell "Voltage" weight=bold; :cell "Current" weight=bold; :cell "Power" weight=bold
  :cell "48 V";                :cell "20 A";               :cell "960 W"
  :cell "24 V";                :cell "10 A";               :cell "240 W"
}

dim1 :dim from=(0, 200) to=(300, 200) {
  :text "300 mm" at=center fill=#666 size=11
}
```
