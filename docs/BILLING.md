# Token and cost accounting

Kuibysheff records every physical provider HTTP attempt, including retries, and
adds an exact monetary report to every `RunOutput`. A missing price is reported
as `unknown`/`partial`; it is never silently converted to zero.

## Precision and evidence

Amounts use decimal arithmetic and serialize as strings. JSON numbers, decimal
strings, and scientific notation are accepted without an intermediate `f64`.
For example, `0.00000894` remains exact when recorded and aggregated.

Cost precision is:

- `actual`: the provider returned the charged amount in a configured unit;
- `calculated`: a versioned catalog or MCP contract calculated the amount from
  provider-reported usage;
- `estimated`: the configured MCP calculator explicitly marked the result as an
  estimate;
- unpriced: no source could establish a price.

The default source order is provider-reported cost, MCP, then local catalog.
Change `billing.source_order` when an enterprise MCP contract must override a
gateway-reported amount.

## Configuration

```yaml
billing:
  provider_id: "openrouter"
  currency: "USD"
  source_order: ["provider_reported", "mcp", "catalog"]
  provider_reported:
    # Unit-less usage.cost/header values are not assumed to be USD.
    unit: "USD"
    json_pointers:
      - "/usage/cost"
      - "/usage/response_cost/total_cost"
    headers: ["x-litellm-response-cost"]
  catalog_path: "./pricing.yaml"
  mcp:
    target: "pricing.calculate_cost"
    timeout_ms: 5000
  on_unpriced: continue

limits:
  max_iterations: 10
  max_tokens: 15000
  max_duration_sec: 120
  max_cost: { amount: "1.00", currency: "USD" }
```

`catalog_path` is relative to the agent config. `billing.mcp.target` references a
dedicated server from the top-level `mcp` list. The billing server is connected
separately and is not advertised to the model or usable by Event-MCP.

An optional billing MCP failure does not fail the run. It produces an unpriced
reason and the resolver continues to the next source. `kbshff check`
still reports connectivity or discovery failures so deployment problems are
visible before a run.

CLI overrides:

```text
kbshff run ... --run-id invoice-row-42 --max-cost USD:0.50
```

## Local catalog

Catalog selection is exact on provider, resolved model, service tier, and the
effective Unix-millisecond window. Ambiguous or expired rules are rejected.

```yaml
version: "2026-08-06"
source: "https://provider.example/pricing"
rules:
  - provider_id: "openai"
    model: "example-model"
    service_tier: "default"
    effective_from_ms: 1785974400000
    effective_until_ms: 1788652800000
    currency: "USD"
    rates:
      input_tokens: { amount: "0.10", per: 1000000 }
      cached_input_tokens: { amount: "0.01", per: 1000000 }
      output_tokens: { amount: "0.40", per: 1000000 }
```

Supported normalized meters include input, cached input, cache write, output,
reasoning/thinking, audio, image, web-search requests, and validated custom
snake-case meters. Rates must describe non-overlapping billable quantities for
the provider contract.

## MCP calculator contract

The host calls the configured tool with no prompt or response body:

```json
{
  "schema_version": "1",
  "target_currency": "USD",
  "request": {
    "attempt": 1,
    "provider_id": "openrouter",
    "requested_model": "example-model",
    "resolved_model": "example-model-2026-08-01",
    "service_tier": "default",
    "request_id": "req-123",
    "usage_reported": true,
    "billable_metrics": {
      "input_tokens": 100,
      "output_tokens": 20
    }
  }
}
```

Preferred `structuredContent`:

```json
{
  "status": "priced",
  "amount": "0.00000894",
  "currency": "USD",
  "precision": "calculated",
  "pricing_version": "contract-2026-08",
  "line_items": [
    { "metric": "input_tokens", "quantity": 100, "amount": "0.000004" }
  ]
}
```

The compatibility form is one JSON text content item. An unpriced result is:

```json
{ "status": "unpriced", "reason": "model is not in the contract" }
```

Malformed output, timeouts, transport failures, unknown targets, and currency
mismatches are fail-soft and become explicit unpriced reasons.

## Output and limits

`RunOutput.usage.cost` always exists:

```json
{
  "run_id": "run-...",
  "usage": {
    "prompt_tokens": 1234,
    "completion_tokens": 321,
    "total_tokens": 1555,
    "cost": {
      "status": "complete",
      "known_total": { "amount": "0.004812", "currency": "USD" },
      "priced_requests": 2,
      "unpriced_requests": 0,
      "budget_status": "enforced",
      "requests": []
    }
  }
}
```

`status: partial` means `known_total` excludes one or more unpriced attempts.
`status: unavailable` means no attempt could be priced. When `max_cost` is set,
`budget_status: degraded` means an unknown attempt prevents strict enforcement.

Like the existing post-response token limit, `max_cost` is checked before the
next model step and immediately after a charged response. One completed request
can therefore cross the limit. No pre-request estimate is claimed as an exact
cap.

ACP has no standard usage field in `PromptResponse`; Kuibysheff emits a final
agent message containing token count and cost (or `unavailable`).

## Reconciliation limitations

- A transport failure or cancellation can have an unknown provider-side charge.
  The attempt remains unpriced until reconciled by an external system.
- Provider-reported credits are not treated as USD unless
  `billing.provider_reported.unit` explicitly maps them.
- Native Anthropic and Gemini HTTP adapters are outside the current scope. Their
  cache/thinking meter categories are represented for compatible gateways and
  future adapters.
- MCP calculator infrastructure costs are not inferred automatically.
