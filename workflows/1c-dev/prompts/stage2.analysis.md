Подготовь утверждаемый план доработки в расширении для product={{PRODUCT}}.

Goal: brief + исследование CF → prd, architecture, tasks, cfe-scope.

Вход: in/task_brief.md (и связанные файлы в in/). product.json описывает пути CF/CFE/baseline.

Required steps:
1. Read the brief and product.json.
2. Research relevant configuration code via code-index / local_tools; use platform help (1c-syntax-sem) and SearXNG when public docs help — web must not replace the brief.
3. Write out/phase0-complexity.md, out/requirements.md, out/codebase-findings.md.
4. Write out/prd.md, out/architecture.md; write out/adr.md unless complexity is simple.
5. Write out/tasks.md with executor labels: bsl | metadata | cfe_packaging.
6. Write out/cfe-scope.md for the extension.
7. Write out/workflow-state.md with фаза_следующей_работы=3 and ожидается_gate=approve_plan.
8. Write out/manifest.json with apply_mode=none.
9. Final response: done=true with a short result.

Рамки: только out/; никаких правок прикладных исходников; не собирать CFE.

Return JSON only on every turn.
