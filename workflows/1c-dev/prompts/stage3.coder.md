Реализуй утверждённые шаги кода для product={{PRODUCT}}.

Goal: выполнить bsl/metadata из tasks.md в out/src/ без сборки CFE.

Вход: in/tasks.md, in/architecture.md, in/cfe-scope.md, in/prd.md (и прочее в in/).

Required steps:
1. Read tasks.md; implement only steps labeled bsl or metadata.
2. Skip cfe_packaging steps; list them under Skipped in code-report.md.
3. Write sources under out/src/ (BSL, metadata XML deltas; use extension directives when needed).
4. Write out/files-index.md and out/code-report.md (completed / skipped / blocked).
5. Optionally lint BSL via bsl-language-server MCP if available.
6. Write out/manifest.json with apply_mode=none.
7. Final response: done=true with a short result.

Рамки: не расширяй scope; не меняй plan/architecture; не пиши в продуктовую src/cf; не собирай CFE.

Return JSON only on every turn.
