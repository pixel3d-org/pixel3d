# Contributing

## Workflow

1. Read `docs/STATUS.md` and pick the next unchecked item of the current phase.
2. Write a failing test first (or a benchmark for performance work), then the smallest implementation.
3. Run `cargo xtask ci-fast`.
4. Update the docs and tick the item in `docs/STATUS.md` in the same commit.
5. Commit. One coherent step per commit; never mix unrelated changes.

Decisions that are not covered by the existing docs are recorded as short ADRs in `docs/adr/NNNN-title.md`.

## Commits

- Conventional Commits: `type(scope): summary`, imperative mood, at most 72 characters, no trailing period. Types: `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `build`, `ci`, `chore`. The scope is the crate or area (`mesh`, `uv`, `mcp`, `xtask`, ...).
- An optional body explains why, wrapped at 72 columns. No trailers.
- Never bypass hooks, never force-push, never rewrite pushed history.

## Code

- Rust stable (pinned in `rust-toolchain.toml`), edition 2024. `unsafe` code is forbidden.
- Libraries never `unwrap`, `expect`, `panic!` or print; they return `thiserror` errors and log with `tracing`. Apps may use `anyhow`.
- Every non-O(1) function documents its time complexity (`/// Complexity: O(n log n) in faces.`). No quadratic loops over mesh elements, objects, keys, texels or pixels; use adjacency and spatial structures.
- Outputs are deterministic: hash map iteration order never reaches an output.
- Internal units are meters and radians; APIs take degrees and unit strings.
- Respect the crate layers (`docs/architecture.md`); `cargo xtask layers` enforces them.
- Never write the em dash (U+2014) or the en dash (U+2013) in code, comments, docs, UI strings or commit messages. Use a colon, a comma, parentheses or a hyphen.
- Dependencies must use a license from the allow list in `deny.toml`.

## Checks

```sh
cargo xtask ci-fast   # before every commit
cargo xtask ci        # everything CI runs (needs cargo-deny)
```
