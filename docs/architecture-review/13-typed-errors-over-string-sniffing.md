# 13 — Typed errors instead of string sniffing

**Status:** open
**Severity:** P1
**Area:** `src/mcp/http_client.rs`, `src/access/mod.rs`, `src/access/protected.rs`
**Rust-skills:** `type-no-stringly`, `err-custom-type`, `pat-matches-macro`, `err-result-over-panic`
**Review:** [review-2026-09.md](review-2026-09.md)

## Problem

Two production paths decide control flow by searching an error's display text. A message that happens to contain the searched substring takes the wrong branch. A third site panics on a parse that the type system could make impossible. The MCP client also advertises a stale crate version.

## Evidence

Legacy HTTP+SSE fallback matches digits inside the protocol error string. Any server message containing `400`, `404`, or `405` selects the legacy transport:

```rust
// src/mcp/http_client.rs
Err(Error::Protocol { error, .. })
    if error.contains("400") || error.contains("404") || error.contains("405") =>
{
    self.try_legacy_http_sse().await
}
```

Workspace validation treats one phrase as fatal and swallows every other failure from the same call:

```rust
// src/access/mod.rs
if reason.contains("must not grant protected") {
    return Err(AccessError::Validation(reason));
}
```

`validate_workspace_excludes_protected` (`src/access/protected.rs:30`) returns a `String` reason, so the caller has nothing else to match on.

Known builtins are parsed from `&'static str` and then asserted:

```rust
// src/access/mod.rs
fn parse_known_builtins(names: impl IntoIterator<Item = &'static str>) -> BTreeSet<QualifiedTool> {
    names
        .into_iter()
        .map(|name| {
            QualifiedTool::parse(name).unwrap_or_else(|reason| {
                unreachable!("known builtin `{name}` must parse: {reason}")
            })
        })
        .collect()
}
```

`clientInfo.version` is hard-coded `"0.1.0"` at `src/mcp/http_client.rs:165` while `Cargo.toml` is `0.2.1`. The same literal appears again around `src/mcp/http_client.rs:250`.

## Acceptance

- [ ] HTTP status fallback matches `StatusCode` (or a dedicated `Error::HttpStatus { status, .. }`), not substrings of a message.
- [ ] `validate_workspace_excludes_protected` returns a typed error; the protected-grant case is a variant, not a phrase.
- [ ] `parse_known_builtins` builds `QualifiedTool` from registry data so a bad static name does not need `unreachable!`.
- [ ] MCP `clientInfo.version` is `env!("CARGO_PKG_VERSION")`.
- [ ] Existing HTTP and access tests still pass; add one case where a protocol message contains `"404"` but the status is not 404, and the client does not switch transport.

## Suggested approach

1. Split transport failures in `mcp::Error` so HTTP status is a field. Keep the human message for logs only.
2. Change `validate_workspace_excludes_protected` to an enum (`ProtectedGrant`, and whatever non-fatal outcomes the current `Ok`-on-other-strings path is papering over). Update the caller to match the variant.
3. Have `BUILTINS` store an already-parsed `QualifiedTool` (or server + tool parts) so `parse_known_builtins` disappears.
4. Replace both `"0.1.0"` client-info literals.

## Related

- [06](06-tool-descriptor-registry.md) — registry is the source of builtin names.
- [14](14-break-remaining-module-cycles.md) — `QualifiedTool` ownership may move with the registry.
