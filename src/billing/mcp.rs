//! Optional host-owned MCP request-cost resolver.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::time::timeout;
use tracing::warn;

use super::{
    decimal_from_value, validate_currency, BillableMetric, BillingError, CostLineItem,
    CostPrecision, CostResolution, CostResolver, Money, ProviderAttemptAccounting,
};
use crate::logging::SharedEventSink;
use crate::mcp::McpRegistry;

/// Fail-soft cost resolver backed by one discovered MCP tool.
pub struct McpCostResolver {
    registry: Arc<McpRegistry>,
    server: String,
    tool: String,
    target: String,
    target_currency: String,
    timeout: Duration,
    logger: Option<SharedEventSink>,
}

impl McpCostResolver {
    /// Builds a resolver after the target has been split and discovered.
    ///
    /// # Errors
    ///
    /// Returns [`BillingError::InvalidCurrency`] when `target_currency` is not a
    /// compact ASCII currency/unit identifier.
    pub fn new(
        registry: Arc<McpRegistry>,
        server: impl Into<String>,
        tool: impl Into<String>,
        target_currency: impl Into<String>,
        timeout_ms: u64,
        logger: Option<SharedEventSink>,
    ) -> Result<Self, BillingError> {
        let server = server.into();
        let tool = tool.into();
        let target_currency = target_currency.into();
        validate_currency(&target_currency)?;
        Ok(Self {
            target: format!("{server}.{tool}"),
            registry,
            server,
            tool,
            target_currency,
            timeout: Duration::from_millis(timeout_ms),
            logger,
        })
    }

    async fn resolve_inner(&self, attempt: &ProviderAttemptAccounting) -> CostResolution {
        let arguments = json!({
            "schema_version": "1",
            "target_currency": self.target_currency,
            "request": attempt,
        });
        let call = self
            .registry
            .call_billing_handler(&self.server, &self.tool, arguments);
        let response = match timeout(self.timeout, call).await {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                return CostResolution::Unpriced {
                    reason: error.to_string(),
                };
            }
            Err(_) => {
                return CostResolution::Unpriced {
                    reason: format!(
                        "MCP calculator timed out after {} ms",
                        self.timeout.as_millis()
                    ),
                };
            }
        };
        let value = match call_tool_payload(response) {
            Ok(value) => value,
            Err(reason) => return CostResolution::Unpriced { reason },
        };
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if status == "unpriced" {
            return CostResolution::Unpriced {
                reason: value
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("MCP calculator returned unpriced")
                    .to_string(),
            };
        }
        if status != "priced" {
            return CostResolution::Unpriced {
                reason: "MCP result status must be `priced` or `unpriced`".to_string(),
            };
        }
        let Some(amount_value) = value.get("amount") else {
            return CostResolution::Unpriced {
                reason: "MCP priced result has no amount".to_string(),
            };
        };
        let amount = match decimal_from_value(amount_value) {
            Ok(amount) => amount,
            Err(error) => {
                return CostResolution::Unpriced {
                    reason: error.to_string(),
                };
            }
        };
        let currency = value
            .get("currency")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if currency != self.target_currency {
            return CostResolution::Unpriced {
                reason: format!(
                    "MCP currency `{currency}` differs from target `{}`",
                    self.target_currency
                ),
            };
        }
        let money = match Money::new(amount, currency) {
            Ok(money) => money,
            Err(error) => {
                return CostResolution::Unpriced {
                    reason: error.to_string(),
                };
            }
        };
        let precision = match value.get("precision").and_then(Value::as_str) {
            Some("actual") => CostPrecision::Actual,
            Some("estimated") => CostPrecision::Estimated,
            Some("calculated") | None => CostPrecision::Calculated,
            Some(other) => {
                return CostResolution::Unpriced {
                    reason: format!("unknown MCP precision `{other}`"),
                };
            }
        };
        let line_items = match parse_line_items(&value) {
            Ok(items) => items,
            Err(reason) => return CostResolution::Unpriced { reason },
        };
        CostResolution::Priced {
            money,
            source: format!("mcp:{}", self.target),
            precision,
            pricing_version: value
                .get("pricing_version")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            line_items,
        }
    }

    async fn audit(
        &self,
        attempt: &ProviderAttemptAccounting,
        resolution: &CostResolution,
        duration_ms: u128,
    ) {
        let Some(logger) = &self.logger else {
            return;
        };
        let (priced, reason) = match resolution {
            CostResolution::Priced { .. } => (true, None),
            CostResolution::Unpriced { reason } => (false, Some(reason.as_str())),
        };
        if let Err(error) = logger
            .write_event(
                "billing_mcp_resolution",
                json!({
                    "target": self.target,
                    "request_id": attempt.request_id,
                    "model": attempt.model_for_pricing(),
                    "priced": priced,
                    "reason": reason,
                    "duration_ms": duration_ms,
                }),
            )
            .await
        {
            warn!(error = ?error, "billing MCP audit write failed");
        }
    }
}

#[async_trait]
impl CostResolver for McpCostResolver {
    fn name(&self) -> &str {
        "mcp"
    }

    async fn resolve(&self, attempt: &ProviderAttemptAccounting) -> CostResolution {
        let started = Instant::now();
        let resolution = self.resolve_inner(attempt).await;
        self.audit(attempt, &resolution, started.elapsed().as_millis())
            .await;
        resolution
    }
}

fn call_tool_payload(response: Value) -> Result<Value, String> {
    if response
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err("MCP CallToolResult set isError=true".to_string());
    }
    if let Some(structured) = response.get("structuredContent") {
        return Ok(structured.clone());
    }
    let content = response
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| "expected structuredContent or one JSON text content item".to_string())?;
    let texts: Vec<_> = content
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect();
    let [text] = texts.as_slice() else {
        return Err("compatibility result must contain exactly one text item".to_string());
    };
    serde_json::from_str(text).map_err(|error| format!("text content is not JSON: {error}"))
}

fn parse_line_items(value: &Value) -> Result<Vec<CostLineItem>, String> {
    let Some(items) = value.get("line_items") else {
        return Ok(Vec::new());
    };
    let items = items
        .as_array()
        .ok_or_else(|| "line_items must be an array".to_string())?;
    items
        .iter()
        .map(|item| {
            let metric = item
                .get("metric")
                .and_then(Value::as_str)
                .ok_or_else(|| "line item metric must be a string".to_string())?
                .parse::<BillableMetric>()
                .map_err(|error| error.to_string())?;
            let quantity = item
                .get("quantity")
                .and_then(Value::as_u64)
                .ok_or_else(|| "line item quantity must be an unsigned integer".to_string())?;
            let amount = item
                .get("amount")
                .ok_or_else(|| "line item amount is required".to_string())
                .and_then(|value| decimal_from_value(value).map_err(|error| error.to_string()))?;
            Ok(CostLineItem {
                metric,
                quantity,
                amount,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::limits::TokenUsage;

    fn attempt() -> ProviderAttemptAccounting {
        ProviderAttemptAccounting {
            attempt: 1,
            provider_id: "demo".to_string(),
            requested_model: "cheap".to_string(),
            resolved_model: None,
            service_tier: None,
            request_id: Some("req-1".to_string()),
            http_status: Some(200),
            occurred_at_ms: 1,
            usage_reported: true,
            usage: TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
            },
            billable_metrics: BTreeMap::new(),
            reported_cost: None,
            error: None,
        }
    }

    #[tokio::test]
    async fn structured_mcp_cost_is_parsed_exactly() {
        let registry = Arc::new(McpRegistry::with_stub_server(
            "pricing",
            "calculate",
            json!({
                "structuredContent": {
                    "status": "priced",
                    "amount": 0.00000894,
                    "currency": "USD",
                    "pricing_version": "contract-1"
                }
            }),
            None,
        ));
        let resolver = McpCostResolver::new(registry, "pricing", "calculate", "USD", 100, None)
            .expect("resolver");

        let CostResolution::Priced { money, .. } = resolver.resolve(&attempt()).await else {
            panic!("expected priced");
        };
        assert_eq!(money.amount().to_string(), "0.00000894");
    }

    #[tokio::test]
    async fn malformed_mcp_result_degrades_to_unpriced() {
        let registry = Arc::new(McpRegistry::with_stub_server(
            "pricing",
            "calculate",
            json!({"structuredContent": {"status": "priced"}}),
            None,
        ));
        let resolver = McpCostResolver::new(registry, "pricing", "calculate", "USD", 100, None)
            .expect("resolver");

        assert!(matches!(
            resolver.resolve(&attempt()).await,
            CostResolution::Unpriced { .. }
        ));
    }
}
