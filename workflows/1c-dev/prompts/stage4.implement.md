Упакуй код кодера в структуру расширения для product={{PRODUCT}}.

Goal: out/cfe/ apply-ready по cfe-scope.md без новой бизнес-логики.

Вход: in/cfe-scope.md, in/coder/ (sources + code-report + files-index), plan files in in/.

Required steps:
1. Read cfe-scope.md and coder sources under in/coder/.
2. Build hierarchical XML extension under out/cfe/ (borrows, Composition, correct paths).
3. Cross-check against scope/baseline rules; do not duplicate exports already on Release/IB.
4. Only trivial syntax/structure fixups — no new business logic. Gaps → implement-report.md.
5. Write out/implement-report.md and out/checklist.md (CheckConfig / staging).
6. Write out/manifest.json with apply_mode=copy_out and files_written.
7. Final response: done=true with a short result.

Рамки: не придумывай фичи; не вызывай BuildCfe/загрузку в ИБ (это оркестратор).

Return JSON only on every turn.
