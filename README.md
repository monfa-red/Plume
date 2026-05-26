# Plume

A small, human-readable language for plain-text diagrams. Flex/grid layout when you want it, composable primitives when you don't, CSS-driven theming throughout. Compiles to clean SVG.

See [SPEC.md](SPEC.md) for the language reference and [IMPLEMENTATION.md](IMPLEMENTATION.md) for the build plan.

## Status

Pre-1.0. The language spec is frozen; the compiler is under construction.

## Usage

```bash
plume diagram.plume -o diagram.svg
```

## Development

```bash
cargo build
cargo test
cargo run -- samples/hello.plume
```

## License

MIT
