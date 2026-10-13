# AGENTS.md

## Commit Messages

The format is `<type>: <description>`: one line, in English, imperative mood, no more than 50 characters, no trailing punctuation.

Common `<type>` values: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`, `revert`...

Example: `fix: keep wrapped linear rows one fixed size`

## Branches

| Branch | Purpose                                                  |
|--------|----------------------------------------------------------|
| `dev`  | Local-only development branch; never pushed to a remote. |
| `main` | Stable branch, kept generally usable.                    |

- All development commits go to `dev` by default.
- A `push` request means: merge `dev` into `main`, then push `main`.
- After every merge into `main`, recreate `dev` fresh from `main`.
- Merging branches must follow the format `merge: <description>`

## Code Style

- `cargo fmt` is always authoritative. Run it after every change and never hand-format against it.
- Indentation within a function, counting the level the function itself introduces, must not exceed three levels; two levels is preferred.
- Keep single-line `if` as `if`; only use `match { true/false }` when it spans multiple lines.
- Do not split logic into many small helper methods; keep a routine cohesive.
- Document intent in doc comments, not in the body. A doc comment opens with a single summary paragraph describing purpose and use, without implementation detail.
