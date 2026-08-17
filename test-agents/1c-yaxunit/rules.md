# 1c-yaxunit rules

- Read `in/agreements-protocol.md` first. Identifiers are literals.
- Missing `in/agreements.json` or `in/agreements.md` → `blocked`. Do not invent names.
- Read `in/docs/` before writing any BSL. Do not invent YAxUnit API.
- Use `expect.yaxunit.procedure` and `expect.test_contains` / `agreements.md` as exact BSL identifiers. Do not rename the gate procedure. Extra tests are allowed.
- No synonyms, translation, or “clearer” names.
- Assert the effect of `Записать`, not a disconnected local `Отказ`.
- SearXNG: only `bia-technologies.github.io/yaxunit` and `github.com/bia-technologies/yaxunit`. Cite URLs in `test-report.md`. Never use ITS or closed Infostart as requirements.
- Write only under `out/` and `notes/`. Never modify product CF.
- Do not implement the feature under test. Tests must describe expected behavior from the plan (fail on baseline).
- Registration only inside `ИсполняемыеСценарии`. Test data in test procedures (or `ЮТест.Данные()`), not in registration.
- Prefer `ДобавитьСерверныйТест` + `ВТранзакции` for document write/posting.
- `done=true` only after a verification table in `test-report.md`.

# Deliverables

- `out/tests/**/*.bsl`
- `out/cfe-tests/` (minimal CFE tree)
- `out/test-report.md` (docs sources + procedure list + verification table)
- `out/manifest.json` (`apply_mode: none`)

# Response protocol

- Exactly one JSON object per reply.
- `done=true` only after deliverables exist (or a blocked report).
