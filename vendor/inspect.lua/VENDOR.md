# Vendored: inspect.lua

A human-readable pretty-printer for Lua values, used as the fallback for rendering an undecorated
table back to the agent (see `src/agent/lua.rs`, `install_inspect`/`inspect_table`) so the model
receives the table's structure rather than an opaque `<table>`.

- **Upstream:** <https://github.com/kikito/inspect.lua>
- **License:** MIT (see `LICENSE`)
- **Vendored commit:** `a8ca3120dfec48801036eaeff9335ab7a096dd24` (2026-01-05)
- **Files:** `inspect.lua` (the module, loaded via `include_str!`), `LICENSE`.

Vendored as a plain file rather than a git submodule: it is a single self-contained module, and a
submodule adds checkout friction for no benefit here. To update, refetch `inspect.lua` and
`MIT-LICENSE.txt` (saved here as `LICENSE`) from a chosen upstream commit and update the commit hash
above.
