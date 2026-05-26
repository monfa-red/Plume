# Working with this user

## Communication

- **Keep replies short.** No preamble, no recap, no closing summaries. One concise sentence or a small table beats a paragraph.
- Don't narrate intentions — just do the thing and report what actually changed.
- For exploratory questions, suggest one path and the main tradeoff in 2–3 lines. Don't decide and implement; propose, then wait.
- Ask before risky/irreversible actions (force push, destructive ops, anything affecting shared state). Local edits are free to make.
- No emojis unless explicitly requested. Plain Markdown. Tables over bullet lists when comparing options.

## Code style

- **No `unsafe`.** If a problem seems to need it, find another path or surface the question rather than reaching for it.
- Standard idioms over clever code. Readability beats LOC count.
- Modular: one concept per file. Split a module when it crosses ~500 LOC.
- Don't fight the formatter or linter — follow whatever `rustfmt.toml` / `clippy.toml` is in the repo.
- Don't add features, validation, or error handling beyond what the task requires.
- Default to no comments. Comments only for non-obvious *why*, never for *what*.

## Testing

- Snapshot tests with `insta` for any output-shaped code.
- Sample input files live in `samples/`. Each spec feature should have at least one sample.
- For SVG renders, verify visually by converting to PNG (via `resvg` CLI) and reading the image. Don't rely on the user to spot-check every iteration.

## Git

- Descriptive commit messages — what changed and why, briefly. Body if needed.
- One purposeful change per commit; don't bundle unrelated edits.
- Don't push to `main` directly when a PR workflow is in use (defer to user).
- Never include "Co-Authored-By" lines.

## After context compaction (or fresh session)

Re-orient by reading, in order:

1. `SPEC.md` — source of truth for the language.
2. `IMPLEMENTATION.md` — current plan, locked decisions, open questions.
3. `git log --oneline -20` — recent progress.

Then continue from the most recent sprint.
