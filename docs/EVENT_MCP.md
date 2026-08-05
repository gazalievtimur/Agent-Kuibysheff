# Event-MCP

## Status

This document defines the Event-MCP contract version `1`. The first implementation is a
synchronous, ordered middleware pipeline built on standard MCP `tools/call`. It does not add a
transport or a non-standard JSON-RPC method.

## Goals

Event-MCP lets explicitly trusted MCP handlers observe, validate, or transform information at
stable stages of an agent run. Typical uses include:

- compacting the provider context before a completion request;
- repairing or validating a model response before directive parsing;
- formatting or validating the final result;
- later, enriching image or other media inputs for text-only models.

Ordinary model-selected MCP tools and Event-MCP handlers are separate capabilities. A handler is
invoked because it is bound in `event_mcp`, not because the model selected it. Binding a handler
does not expose that tool to the model or bypass the existing skills/tool policy.

## Execution model

Each event has an ordered list of handlers. The list position is authoritative and is independent
of MCP server connection order or map iteration order.

1. The dispatcher creates an `EventEnvelope` with the current payload.
2. It invokes the first handler through standard MCP `tools/call`.
3. `pass` keeps the current payload; `replace` makes the returned payload the input to the next
   handler.
4. `reject` stops the chain and aborts the run.
5. A technical failure follows that handler's `on_error` policy:
   - `continue`: retain the last valid payload and invoke the next handler;
   - `abort`: stop the chain and abort the run.

Handlers run sequentially. Parallel fan-out is intentionally excluded because merging concurrent
mutations would make ordering and failure semantics ambiguous.

Example:

```yaml
event_mcp:
  events:
    context.before_model:
      handlers:
        - id: redact-secrets
          target: security.redact
          timeout_ms: 3000
          on_error: abort
        - id: compact-history
          target: context.compact
          timeout_ms: 5000
          on_error: continue
```

The output of `security.redact` is therefore the input of `context.compact`.

## Wire contract

The dispatcher passes one object as the MCP tool arguments:

```json
{
  "spec_version": "1",
  "event_id": "context.before_model:2",
  "event": "context.before_model",
  "iteration": 2,
  "payload": {}
}
```

- `spec_version`, `event_id`, `event`, and `iteration` are host-owned metadata.
- `payload` is event-specific data and is the only replaceable value.
- `iteration` is omitted for run-level events.

The handler returns an MCP `CallToolResult`. The preferred response is its
`structuredContent` object:

```json
{ "action": "replace", "payload": {} }
```

For compatibility, a single text content item containing exactly one JSON object is also accepted.
The outcome schema is strict:

- `{ "action": "pass" }`
- `{ "action": "replace", "payload": ... }`
- `{ "action": "reject", "reason": "..." }`

Unknown fields, a missing replacement payload, or a malformed compatibility response are protocol
errors and use the configured technical failure policy.

## Event catalogue

The architecture reserves these stable stages:

- `run.input`: transform normalized user input and attachments before prompt assembly;
- `context.before_model`: transform the message snapshot immediately before a provider request;
- `model.after_response`: transform provider text before audit and directive parsing;
- `directive.after_parse`: validate or transform a parsed directive;
- `tool.before_call`: validate or transform tool arguments;
- `tool.after_call`: validate or transform a successful tool result;
- `tool.on_error`: observe or translate a tool failure;
- `run.before_output`: validate or transform the final user-facing result;
- `run.on_error`: observe or translate a terminal run failure.

The MVP implements `context.before_model`, `model.after_response`, and `run.before_output`.

### MVP payloads

`context.before_model`:

```json
{
  "messages": [
    { "role": "system", "content": "..." },
    { "role": "user", "content": "..." }
  ]
}
```

The transformed messages are a provider-only snapshot. Canonical history remains unchanged. The
host rejects an empty list, a changed first role, or removal of the original system message.

`model.after_response`:

```json
{ "content": "..." }
```

Token usage is host metadata and cannot be changed by a handler.

`run.before_output`:

```json
{ "result": "..." }
```

Only the result text is replaceable; usage, stop reason, and log report remain host-owned.

## Timeouts, cancellation, and limits

Each handler has a positive `timeout_ms`. Dispatcher waits are cancellation-aware and also remain
inside the run's wall-clock deadline. Request and response JSON are bounded before they are sent or
accepted. A timeout, cancellation, malformed outcome, unavailable target, or MCP transport error
is a technical failure.

Cancellation always terminates the chain even if a handler is configured with
`on_error: continue`.

## Security and observability

An Event-MCP binding is an explicit trust grant to send that event's payload to the target MCP
server. Configuration validation fails if the target was not discovered through `tools/list`.

Event calls use metadata-only audit records by default: event, handler id, target, input/output
sizes, duration, action, and success. Prompt and response bodies are not written to the MCP audit
log. Existing model-selected `mcp_tool_call` audit behavior is unchanged.

Handlers cannot change event metadata, tool policy, token accounting, stop reason, or log paths.

## Media and vision extension

Native image handling is outside the MVP because current ACP ingestion and provider messages are
text-only. A later `run.input` adapter should introduce typed content parts such as:

```json
{
  "parts": [
    { "type": "text", "text": "..." },
    {
      "type": "image_ref",
      "media_type": "image/png",
      "uri": "host-authorized-reference"
    }
  ]
}
```

An OCR or vision handler may replace an `image_ref` with an `enrichment` part containing extracted
text and provenance. The host, not the handler, resolves local paths or bytes after access-policy
checks. This prevents a media plugin from turning an arbitrary path into an implicit filesystem
capability.

## Compatibility

With `event_mcp` absent or with no configured events, the dispatcher is a no-op and existing runs,
MCP tools, access policy, and configuration behavior are unchanged.
