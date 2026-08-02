# 01 — MCP stdio: NDJSON framing вместо Content-Length

**Status:** done  
**Severity:** P0  
**Area:** `src/mcp/stdio_client.rs`  
**Rust-skills:** `api-parse-dont-validate`, `err-custom-type`, `test-integration-dir`

## Problem

Клиент пишет и читает LSP-style framing:

```text
Content-Length: N\r\n\r\n{json}
```

Спецификация MCP stdio требует **newline-delimited JSON-RPC** (одно сообщение = одна строка, без embedded newlines). Официальные SDK (TypeScript/Python/Go) используют NDJSON. Текущий клиент несовместим со стандартными MCP-серверами.

## Evidence

- Write path: `src/mcp/stdio_client.rs` (~394) — `Content-Length: {}\r\n\r\n`
- Read path: тот же файл (~402–429) — парсинг `content-length` header
- Spec: https://modelcontextprotocol.io/specification/2025-11-25/basic/transports#stdio

## Acceptance

- [x] Encode: `serde_json::to_vec` + `\n`, без headers
- [x] Decode: читать построчно до `\n`, парсить JSON-RPC
- [x] Integration test против compliant NDJSON server (fixture / mock)
- [x] Regression: заведомо Content-Length ответ → явная protocol error

## Suggested approach

1. Заменить `write_message` / `read_message` на line-based I/O.
2. Ограничить max line size (DoS guard).
3. Добавить `tests/` interop-фикстуру (Node или Rust echo-server).
4. Обновить CONTRACT/README, если там упоминается framing.

## Notes

Не путать с HTTP MCP (`http_client.rs`) — там framing другой и в целом ближе к Streamable HTTP.
