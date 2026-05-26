# Plume

A small, human-readable language for plain-text diagrams. Flex/grid layout when you want it, composable primitives when you don't, CSS-driven theming throughout. Compiles to clean SVG.

See [SPEC.md](SPEC.md) for the language reference and [IMPLEMENTATION.md](IMPLEMENTATION.md) for the build plan.

## Status

Pre-1.0. The language spec is frozen; the compiler is under construction.

## Install

```bash
cargo install --path .
```

## Usage

```bash
plume diagram.plume -o diagram.svg
```

Read from stdin:

```bash
cat diagram.plume | plume -
```

### CLI flags

| Flag | Meaning |
|---|---|
| `-o, --output FILE` | Output path (default: stdout). |
| `--format svg\|html` | Output as raw SVG (default) or wrapped in a minimal HTML page. |
| `--standalone` | Force-embed the default `<style>` block (the default behaviour; accept for spec compliance). |
| `--no-defaults` | Omit the default `<style>` block — the host page is expected to supply `--plume-*` custom properties. |
| `--bake-vars` | Emit `var()` values inline as their resolved literal. Required for renderers without CSS-variable support (resvg, librsvg, raster converters, email clients). |
| `--check` | Parse and validate only — no layout, no render. |
| `--theme FILE` | CSS file with `--plume-*` overrides. Layout-affecting overrides bake into the layout; visual overrides flow into the emitted `<style>` block. |
| `-h, --help`, `-V, --version` | Standard. |

Exit codes: `0` success, `1` parse/resolve error, `2` I/O error, `3` invalid CLI arguments.

### Examples

```bash
# Validate only
plume --check diagram.plume

# Inline literal colours for raster pipelines
plume --bake-vars diagram.plume -o diagram.svg && resvg diagram.svg diagram.png

# Theme + HTML wrapper
plume --theme dark.css --format html diagram.plume -o diagram.html
```

A complete diagram:

```
defaults { gap=24 accent=#0a84ff }

styles { bold weight=bold }

shapes {
  psu :rect radius=5
}

scene layout=row gap=40 {
  outlet :oval "Outlet"
  drive  :psu  "Drive PSU" .bold
}

wires { outlet -> drive }
```

## Development

```bash
cargo build
cargo test
cargo run -- samples/hello.plume
```

## License

MIT
