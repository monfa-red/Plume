# Next session — wire spacing bug

## The bug

In `samples/wires_realistic.plume` (user's modified version with solid
`.quiet` and `<-` op), three wires arrive at **Dog**:

- `bowl → dog × 2` (red, two siblings of one bundle of size 2)
- `water → dog` (blue, separate decl)

User reports that the blue wire's spacing to the red wires is smaller than
the configured `|wire| gap:N`. Visually the blue wire's vertical segment
runs close to bowl→dog's vertical segment in a way that doesn't respect
the gap.

User also reports that setting `|scene| gap:N` doesn't change wire spacing.
They expected it to.

## Don't trust the previous session's diagnosis

The previous session attempted a fix (commit `c3bff86`) that the user said
was worse. It was reverted with `git reset --hard 6aeaf49 && git push
--force-with-lease`. Don't repeat what that commit did.

That session also wrote speculation about what the cause might be. **Ignore
that speculation.** The previous session couldn't find the real issue.
Look at the code fresh, reproduce the bug yourself, and trace what's
actually happening before forming a theory.

## How to reproduce

```
cargo run -- samples/wires_realistic.plume --bake-vars -o /tmp/wr.svg
resvg /tmp/wr.svg /tmp/wr.png
# Read /tmp/wr.png with the Read tool
# Inspect /tmp/wr.svg paths with grep for the wire coordinates
```

The wires of interest are everything `data-to="garden.dog"`.

## How to verify a fix

- All three wires arriving at Dog should be at least `gap = 12` apart at
  every point, including their vertical segments approaching the shape.
- All other samples must still render correctly. Re-run
  `cargo test` and visually check `samples/wires_realistic.plume`,
  `samples/full_example.plume`, `samples/wires_fan.plume`,
  `samples/wires_bus.plume`.
- The user will verify visually. Don't claim it's fixed without their
  confirmation.

## Repo state at handoff

- `main` is at `6aeaf49` (previous good state) + the handoff commit.
- 89 tests pass. clippy + fmt clean.
- `samples/wires_realistic.plume` is in the user's modified form (solid
  `.quiet`, `<-` op, `gap:12`). Leave it as-is.

## Don't repeat

- Previous session shipped a fix based on a partial trace, the user spotted
  the same bug in a slightly different layout, and we reverted. Don't ship
  before the user confirms the rendered output.
