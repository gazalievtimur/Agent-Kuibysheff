//! Exact provider-usage accounting and monetary cost resolution.
//!
//! Monetary amounts are decimal strings on the wire. Provider JSON numbers are
//! converted directly from their preserved lexical representation, never through
//! `f64`.

mod mcp;

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use thiserror::Error;

use crate::limits::TokenUsage;

pub use mcp::McpCostResolver;

/// Billing-domain failures.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BillingError {
    #[error("invalid decimal amount `{0}`")]
    InvalidDecimal(String),
    #[error("invalid currency or unit `{0}`")]
    InvalidCurrency(String),
    #[error("failed to read pricing catalog `{path}`: {source}")]
    ReadCatalog {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse pricing catalog: {0}")]
    ParseCatalog(String),
    #[error("invalid pricing catalog: {0}")]
    InvalidCatalog(String),
    #[error("decimal arithmetic overflow")]
    ArithmeticOverflow,
}

/// Exact decimal money/credit amount with an explicit ISO-4217 currency or custom unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Money {
    pub amount: Decimal,
    pub currency: String,
}

impl Money {
    /// Builds a validated monetary amount.
    ///
    /// # Errors
    ///
    /// Returns [`BillingError::InvalidCurrency`] when `currency` is not a compact
    /// ASCII currency/unit identifier.
    pub fn new(amount: Decimal, currency: impl Into<String>) -> Result<Self, BillingError> {
        let currency = currency.into();
        validate_currency(&currency)?;
        Ok(Self { amount, currency })
    }

    /// Parses a decimal amount without a floating-point intermediate.
    ///
    /// # Errors
    ///
    /// Returns [`BillingError::InvalidDecimal`] for malformed or out-of-range values.
    pub fn parse(amount: &str, currency: impl Into<String>) -> Result<Self, BillingError> {
        let amount = parse_decimal(amount)?;
        Self::new(amount, currency)
    }

    /// Adds two amounts only when their currencies match.
    ///
    /// # Errors
    ///
    /// Returns an error for a currency mismatch or decimal overflow.
    pub fn checked_add(&self, other: &Self) -> Result<Self, BillingError> {
        if self.currency != other.currency {
            return Err(BillingError::InvalidCurrency(format!(
                "cannot add {} to {}",
                other.currency, self.currency
            )));
        }
        let amount = self
            .amount
            .checked_add(other.amount)
            .ok_or(BillingError::ArithmeticOverflow)?;
        Self::new(amount, self.currency.clone())
    }
}

impl FromStr for Money {
    type Err = BillingError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (currency, amount) = value.split_once(':').ok_or_else(|| {
            BillingError::InvalidDecimal(format!(
                "{value} (expected CURRENCY:AMOUNT, for example USD:1.00)"
            ))
        })?;
        Self::parse(amount, currency)
    }
}

impl Serialize for Money {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct Wire<'a> {
            amount: String,
            currency: &'a str,
        }

        Wire {
            amount: self.amount.to_string(),
            currency: &self.currency,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Money {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            amount: Value,
            currency: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        let amount = decimal_from_value(&wire.amount).map_err(serde::de::Error::custom)?;
        Self::new(amount, wire.currency).map_err(serde::de::Error::custom)
    }
}

fn validate_currency(value: &str) -> Result<(), BillingError> {
    let valid = (3..=24).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(BillingError::InvalidCurrency(value.to_string()))
    }
}

/// Parses decimal, including scientific notation, without going through `f64`.
pub fn parse_decimal(value: &str) -> Result<Decimal, BillingError> {
    let trimmed = value.trim();
    Decimal::from_str_exact(trimmed)
        .or_else(|_| Decimal::from_scientific(trimmed))
        .map_err(|_| BillingError::InvalidDecimal(trimmed.to_string()))
}

/// Converts a JSON string or number to an exact decimal.
///
/// `serde_json` is built with `arbitrary_precision`, so [`Value::Number`] retains
/// the original number lexeme used here.
pub fn decimal_from_value(value: &Value) -> Result<Decimal, BillingError> {
    match value {
        Value::String(text) => parse_decimal(text),
        Value::Number(number) => parse_decimal(&number.to_string()),
        other => Err(BillingError::InvalidDecimal(other.to_string())),
    }
}

/// A normalized meter used by provider price catalogs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum BillableMetric {
    InputTokens,
    CachedInputTokens,
    CacheWriteInputTokens,
    OutputTokens,
    ReasoningTokens,
    AudioInputTokens,
    AudioOutputTokens,
    ImageInputTokens,
    ImageOutputTokens,
    WebSearchRequests,
    Other(String),
}

impl BillableMetric {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::InputTokens => "input_tokens",
            Self::CachedInputTokens => "cached_input_tokens",
            Self::CacheWriteInputTokens => "cache_write_input_tokens",
            Self::OutputTokens => "output_tokens",
            Self::ReasoningTokens => "reasoning_tokens",
            Self::AudioInputTokens => "audio_input_tokens",
            Self::AudioOutputTokens => "audio_output_tokens",
            Self::ImageInputTokens => "image_input_tokens",
            Self::ImageOutputTokens => "image_output_tokens",
            Self::WebSearchRequests => "web_search_requests",
            Self::Other(value) => value,
        }
    }
}

impl fmt::Display for BillableMetric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for BillableMetric {
    type Err = BillingError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let trimmed = value.trim();
        if trimmed.is_empty()
            || !trimmed
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(BillingError::InvalidCatalog(format!(
                "invalid billable metric `{value}`"
            )));
        }
        Ok(match trimmed {
            "input_tokens" => Self::InputTokens,
            "cached_input_tokens" => Self::CachedInputTokens,
            "cache_write_input_tokens" => Self::CacheWriteInputTokens,
            "output_tokens" => Self::OutputTokens,
            "reasoning_tokens" => Self::ReasoningTokens,
            "audio_input_tokens" => Self::AudioInputTokens,
            "audio_output_tokens" => Self::AudioOutputTokens,
            "image_input_tokens" => Self::ImageInputTokens,
            "image_output_tokens" => Self::ImageOutputTokens,
            "web_search_requests" => Self::WebSearchRequests,
            other => Self::Other(other.to_string()),
        })
    }
}

impl Serialize for BillableMetric {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for BillableMetric {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// Cost reported directly by an upstream response body or header.
#[derive(Debug, Clone, Serialize)]
pub struct ReportedCost {
    #[serde(serialize_with = "serialize_decimal")]
    pub amount: Decimal,
    pub unit: Option<String>,
    pub source: String,
    #[serde(serialize_with = "serialize_decimal_map")]
    pub details: BTreeMap<String, Decimal>,
}

/// Accounting captured for one physical HTTP attempt, including retries.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderAttemptAccounting {
    pub attempt: u32,
    pub provider_id: String,
    pub requested_model: String,
    pub resolved_model: Option<String>,
    pub service_tier: Option<String>,
    pub request_id: Option<String>,
    pub http_status: Option<u16>,
    pub occurred_at_ms: u128,
    pub usage_reported: bool,
    pub usage: TokenUsage,
    pub billable_metrics: BTreeMap<BillableMetric, u64>,
    pub reported_cost: Option<ReportedCost>,
    pub error: Option<String>,
}

impl ProviderAttemptAccounting {
    #[must_use]
    pub fn model_for_pricing(&self) -> &str {
        self.resolved_model
            .as_deref()
            .unwrap_or(&self.requested_model)
    }
}

/// Accuracy class of a resolved request cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CostPrecision {
    Actual,
    Calculated,
    Estimated,
}

/// One priced meter line.
#[derive(Debug, Clone, Serialize)]
pub struct CostLineItem {
    pub metric: String,
    pub quantity: u64,
    #[serde(serialize_with = "serialize_decimal")]
    pub amount: Decimal,
}

/// Cost result for one provider HTTP attempt.
#[derive(Debug, Clone, Serialize)]
pub struct RequestCost {
    pub iteration: u32,
    pub attempt: u32,
    pub provider_id: String,
    pub request_id: Option<String>,
    pub model: String,
    pub service_tier: Option<String>,
    pub http_status: Option<u16>,
    pub usage_reported: bool,
    pub usage: TokenUsage,
    pub billable_metrics: BTreeMap<BillableMetric, u64>,
    pub amount: Option<Money>,
    pub source: Option<String>,
    pub precision: Option<CostPrecision>,
    pub pricing_version: Option<String>,
    pub line_items: Vec<CostLineItem>,
    pub reason: Option<String>,
}

/// Resolution returned by one source in the pricing chain.
#[derive(Debug, Clone)]
pub enum CostResolution {
    Priced {
        money: Money,
        source: String,
        precision: CostPrecision,
        pricing_version: Option<String>,
        line_items: Vec<CostLineItem>,
    },
    Unpriced {
        reason: String,
    },
}

/// Pluggable request-cost source.
#[async_trait]
pub trait CostResolver: Send + Sync {
    fn name(&self) -> &str;

    async fn resolve(&self, attempt: &ProviderAttemptAccounting) -> CostResolution;
}

/// Resolver placeholder that preserves why a configured source is unavailable.
pub struct UnavailableCostResolver {
    name: String,
    reason: String,
}

impl UnavailableCostResolver {
    #[must_use]
    pub fn new(name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            reason: reason.into(),
        }
    }
}

#[async_trait]
impl CostResolver for UnavailableCostResolver {
    fn name(&self) -> &str {
        &self.name
    }

    async fn resolve(&self, _attempt: &ProviderAttemptAccounting) -> CostResolution {
        CostResolution::Unpriced {
            reason: self.reason.clone(),
        }
    }
}

/// Ordered fail-soft pricing source chain.
#[derive(Default)]
pub struct CostResolverChain {
    resolvers: Vec<Arc<dyn CostResolver>>,
}

impl CostResolverChain {
    #[must_use]
    pub fn new(resolvers: Vec<Arc<dyn CostResolver>>) -> Self {
        Self { resolvers }
    }

    /// Returns the first priced result, retaining all failure reasons otherwise.
    pub async fn resolve(&self, attempt: &ProviderAttemptAccounting) -> CostResolution {
        let mut reasons = Vec::new();
        for resolver in &self.resolvers {
            match resolver.resolve(attempt).await {
                priced @ CostResolution::Priced { .. } => return priced,
                CostResolution::Unpriced { reason } => {
                    reasons.push(format!("{}: {reason}", resolver.name()));
                }
            }
        }
        CostResolution::Unpriced {
            reason: if reasons.is_empty() {
                "no billing sources configured".to_string()
            } else {
                reasons.join("; ")
            },
        }
    }
}

/// Resolver for provider-returned `usage.cost` / response-cost headers.
pub struct ProviderReportedCostResolver {
    target_currency: String,
    default_unit: Option<String>,
}

impl ProviderReportedCostResolver {
    /// Builds a provider-reported resolver.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid currency/unit identifiers.
    pub fn new(
        target_currency: impl Into<String>,
        default_unit: Option<String>,
    ) -> Result<Self, BillingError> {
        let target_currency = target_currency.into();
        validate_currency(&target_currency)?;
        if let Some(unit) = &default_unit {
            validate_currency(unit)?;
        }
        Ok(Self {
            target_currency,
            default_unit,
        })
    }
}

#[async_trait]
impl CostResolver for ProviderReportedCostResolver {
    fn name(&self) -> &str {
        "provider_reported"
    }

    async fn resolve(&self, attempt: &ProviderAttemptAccounting) -> CostResolution {
        let Some(reported) = &attempt.reported_cost else {
            return CostResolution::Unpriced {
                reason: "provider did not report cost".to_string(),
            };
        };
        let unit = reported.unit.as_ref().or(self.default_unit.as_ref());
        let Some(unit) = unit else {
            return CostResolution::Unpriced {
                reason: "reported amount has no currency/unit mapping".to_string(),
            };
        };
        if unit != &self.target_currency {
            return CostResolution::Unpriced {
                reason: format!(
                    "reported unit `{unit}` cannot be aggregated as `{}` without conversion",
                    self.target_currency
                ),
            };
        }
        match Money::new(reported.amount, unit.clone()) {
            Ok(money) => CostResolution::Priced {
                money,
                source: reported.source.clone(),
                precision: CostPrecision::Actual,
                pricing_version: None,
                line_items: reported
                    .details
                    .iter()
                    .map(|(metric, amount)| CostLineItem {
                        metric: metric.clone(),
                        quantity: 0,
                        amount: *amount,
                    })
                    .collect(),
            },
            Err(error) => CostResolution::Unpriced {
                reason: error.to_string(),
            },
        }
    }
}

/// Versioned deterministic local pricing catalog.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PricingCatalog {
    pub version: String,
    pub source: String,
    pub rules: Vec<PricingRule>,
}

impl PricingCatalog {
    /// Loads YAML or JSON according to file extension/content.
    ///
    /// # Errors
    ///
    /// Returns read, parse, or catalog validation errors.
    pub fn load(path: &Path) -> Result<Self, BillingError> {
        let contents = fs::read_to_string(path).map_err(|source| BillingError::ReadCatalog {
            path: path.display().to_string(),
            source,
        })?;
        let catalog: Self = if path.extension().is_some_and(|ext| ext == "json") {
            serde_json::from_str(&contents)
                .map_err(|error| BillingError::ParseCatalog(error.to_string()))?
        } else {
            serde_yaml::from_str(&contents)
                .map_err(|error| BillingError::ParseCatalog(error.to_string()))?
        };
        catalog.validate()?;
        Ok(catalog)
    }

    /// Validates identifiers, rates, windows, and duplicate rule keys.
    ///
    /// # Errors
    ///
    /// Returns [`BillingError::InvalidCatalog`] when a rule cannot be selected safely.
    pub fn validate(&self) -> Result<(), BillingError> {
        if self.version.trim().is_empty() || self.source.trim().is_empty() {
            return Err(BillingError::InvalidCatalog(
                "version and source must not be empty".to_string(),
            ));
        }
        let mut exact_windows = HashSet::new();
        for (index, rule) in self.rules.iter().enumerate() {
            if rule.provider_id.trim().is_empty() || rule.model.trim().is_empty() {
                return Err(BillingError::InvalidCatalog(
                    "provider_id and model must not be empty".to_string(),
                ));
            }
            validate_currency(&rule.currency)?;
            if rule
                .effective_until_ms
                .is_some_and(|until| until <= rule.effective_from_ms)
            {
                return Err(BillingError::InvalidCatalog(format!(
                    "invalid effective window for {}/{}",
                    rule.provider_id, rule.model
                )));
            }
            if rule.fixed_cost.is_none() && rule.rates.is_empty() {
                return Err(BillingError::InvalidCatalog(format!(
                    "rule {}/{} has no fixed_cost or rates",
                    rule.provider_id, rule.model
                )));
            }
            let key = (
                rule.provider_id.clone(),
                rule.model.clone(),
                rule.service_tier.clone(),
                rule.effective_from_ms,
                rule.effective_until_ms,
            );
            if !exact_windows.insert(key) {
                return Err(BillingError::InvalidCatalog(format!(
                    "duplicate pricing window for {}/{}",
                    rule.provider_id, rule.model
                )));
            }
            for other in self.rules.iter().skip(index + 1) {
                let same_selector = rule.provider_id == other.provider_id
                    && rule.model == other.model
                    && rule.service_tier == other.service_tier;
                let overlaps = rule.effective_from_ms
                    < other.effective_until_ms.unwrap_or(u128::MAX)
                    && other.effective_from_ms < rule.effective_until_ms.unwrap_or(u128::MAX);
                if same_selector && overlaps {
                    return Err(BillingError::InvalidCatalog(format!(
                        "overlapping pricing windows for {}/{} tier {:?}",
                        rule.provider_id, rule.model, rule.service_tier
                    )));
                }
            }
            for rate in rule.rates.values() {
                if rate.per == 0 {
                    return Err(BillingError::InvalidCatalog(
                        "meter rate `per` must be greater than zero".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// One exact provider/model/tier price window.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PricingRule {
    pub provider_id: String,
    pub model: String,
    pub service_tier: Option<String>,
    #[serde(default)]
    pub effective_from_ms: u128,
    pub effective_until_ms: Option<u128>,
    pub currency: String,
    #[serde(default, deserialize_with = "deserialize_optional_decimal")]
    pub fixed_cost: Option<Decimal>,
    #[serde(default)]
    pub rates: BTreeMap<BillableMetric, MeterRate>,
}

/// Price `amount` for each `per` units.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeterRate {
    #[serde(deserialize_with = "deserialize_decimal")]
    pub amount: Decimal,
    pub per: u64,
}

/// Catalog-backed cost resolver.
pub struct CatalogCostResolver {
    catalog: PricingCatalog,
    target_currency: String,
}

impl CatalogCostResolver {
    /// Builds a resolver after validating the catalog.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid catalogs or target currency.
    pub fn new(
        catalog: PricingCatalog,
        target_currency: impl Into<String>,
    ) -> Result<Self, BillingError> {
        catalog.validate()?;
        let target_currency = target_currency.into();
        validate_currency(&target_currency)?;
        Ok(Self {
            catalog,
            target_currency,
        })
    }

    fn matching_rule<'a>(
        &'a self,
        attempt: &ProviderAttemptAccounting,
    ) -> Result<&'a PricingRule, String> {
        let matches: Vec<_> = self
            .catalog
            .rules
            .iter()
            .filter(|rule| {
                rule.provider_id == attempt.provider_id
                    && rule.model == attempt.model_for_pricing()
                    && rule.service_tier.as_deref() == attempt.service_tier.as_deref()
                    && attempt.occurred_at_ms >= rule.effective_from_ms
                    && rule
                        .effective_until_ms
                        .is_none_or(|until| attempt.occurred_at_ms < until)
            })
            .collect();
        match matches.as_slice() {
            [rule] => Ok(rule),
            [] => Err(format!(
                "no effective rule for {}/{} tier {:?}",
                attempt.provider_id,
                attempt.model_for_pricing(),
                attempt.service_tier
            )),
            _ => Err(format!(
                "ambiguous effective rules for {}/{}",
                attempt.provider_id,
                attempt.model_for_pricing()
            )),
        }
    }
}

#[async_trait]
impl CostResolver for CatalogCostResolver {
    fn name(&self) -> &str {
        "catalog"
    }

    async fn resolve(&self, attempt: &ProviderAttemptAccounting) -> CostResolution {
        let rule = match self.matching_rule(attempt) {
            Ok(rule) => rule,
            Err(reason) => return CostResolution::Unpriced { reason },
        };
        if rule.currency != self.target_currency {
            return CostResolution::Unpriced {
                reason: format!(
                    "catalog rule currency `{}` differs from target `{}`",
                    rule.currency, self.target_currency
                ),
            };
        }
        if !attempt.usage_reported && !rule.rates.is_empty() {
            return CostResolution::Unpriced {
                reason: "provider usage missing for metered catalog rule".to_string(),
            };
        }

        let mut total = rule.fixed_cost.unwrap_or(Decimal::ZERO);
        let mut line_items = Vec::new();
        if let Some(fixed) = rule.fixed_cost {
            line_items.push(CostLineItem {
                metric: "request".to_string(),
                quantity: 1,
                amount: fixed,
            });
        }
        for (metric, rate) in &rule.rates {
            let quantity = attempt
                .billable_metrics
                .get(metric)
                .copied()
                .unwrap_or_default();
            let amount = match rate
                .amount
                .checked_mul(Decimal::from(quantity))
                .and_then(|value| value.checked_div(Decimal::from(rate.per)))
            {
                Some(amount) => amount,
                None => {
                    return CostResolution::Unpriced {
                        reason: BillingError::ArithmeticOverflow.to_string(),
                    };
                }
            };
            total = match total.checked_add(amount) {
                Some(total) => total,
                None => {
                    return CostResolution::Unpriced {
                        reason: BillingError::ArithmeticOverflow.to_string(),
                    };
                }
            };
            line_items.push(CostLineItem {
                metric: metric.to_string(),
                quantity,
                amount,
            });
        }

        match Money::new(total, rule.currency.clone()) {
            Ok(money) => CostResolution::Priced {
                money,
                source: format!("catalog:{}", self.catalog.source),
                precision: CostPrecision::Calculated,
                pricing_version: Some(self.catalog.version.clone()),
                line_items,
            },
            Err(error) => CostResolution::Unpriced {
                reason: error.to_string(),
            },
        }
    }
}

/// Completeness of the run-level monetary report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CostReportStatus {
    Complete,
    Partial,
    Unavailable,
}

/// Run-level exact monetary summary and per-attempt ledger.
#[derive(Debug, Clone, Serialize)]
pub struct RunCostReport {
    pub status: CostReportStatus,
    pub known_total: Option<Money>,
    pub priced_requests: u32,
    pub unpriced_requests: u32,
    pub budget_status: String,
    pub requests: Vec<RequestCost>,
}

impl Default for RunCostReport {
    fn default() -> Self {
        Self {
            status: CostReportStatus::Unavailable,
            known_total: None,
            priced_requests: 0,
            unpriced_requests: 0,
            budget_status: "not_configured".to_string(),
            requests: Vec::new(),
        }
    }
}

/// Mutable run-level cost accumulator.
pub struct RunCostTracker {
    target_currency: String,
    known_total: Decimal,
    priced_requests: u32,
    unpriced_requests: u32,
    requests: Vec<RequestCost>,
    max_cost: Option<Money>,
}

impl RunCostTracker {
    /// Creates a tracker for one target currency.
    ///
    /// # Errors
    ///
    /// Returns an error when target/max-cost currencies are invalid or differ.
    pub fn new(
        target_currency: impl Into<String>,
        max_cost: Option<Money>,
    ) -> Result<Self, BillingError> {
        let target_currency = target_currency.into();
        validate_currency(&target_currency)?;
        if let Some(limit) = &max_cost {
            if limit.currency != target_currency {
                return Err(BillingError::InvalidCurrency(format!(
                    "max_cost currency `{}` differs from target `{target_currency}`",
                    limit.currency
                )));
            }
        }
        Ok(Self {
            target_currency,
            known_total: Decimal::ZERO,
            priced_requests: 0,
            unpriced_requests: 0,
            requests: Vec::new(),
            max_cost,
        })
    }

    pub fn record(
        &mut self,
        iteration: u32,
        attempt: &ProviderAttemptAccounting,
        resolution: CostResolution,
    ) {
        match resolution {
            CostResolution::Priced {
                money,
                source,
                precision,
                pricing_version,
                line_items,
            } => {
                if money.currency == self.target_currency {
                    if let Some(total) = self.known_total.checked_add(money.amount) {
                        self.known_total = total;
                        self.priced_requests = self.priced_requests.saturating_add(1);
                        self.requests.push(RequestCost {
                            iteration,
                            attempt: attempt.attempt,
                            provider_id: attempt.provider_id.clone(),
                            request_id: attempt.request_id.clone(),
                            model: attempt.model_for_pricing().to_string(),
                            service_tier: attempt.service_tier.clone(),
                            http_status: attempt.http_status,
                            usage_reported: attempt.usage_reported,
                            usage: attempt.usage,
                            billable_metrics: attempt.billable_metrics.clone(),
                            amount: Some(money),
                            source: Some(source),
                            precision: Some(precision),
                            pricing_version,
                            line_items,
                            reason: None,
                        });
                        return;
                    }
                }
                self.record_unpriced(iteration, attempt, "currency mismatch or total overflow");
            }
            CostResolution::Unpriced { reason } => {
                self.record_unpriced(iteration, attempt, reason);
            }
        }
    }

    fn record_unpriced(
        &mut self,
        iteration: u32,
        attempt: &ProviderAttemptAccounting,
        reason: impl Into<String>,
    ) {
        self.unpriced_requests = self.unpriced_requests.saturating_add(1);
        self.requests.push(RequestCost {
            iteration,
            attempt: attempt.attempt,
            provider_id: attempt.provider_id.clone(),
            request_id: attempt.request_id.clone(),
            model: attempt.model_for_pricing().to_string(),
            service_tier: attempt.service_tier.clone(),
            http_status: attempt.http_status,
            usage_reported: attempt.usage_reported,
            usage: attempt.usage,
            billable_metrics: attempt.billable_metrics.clone(),
            amount: None,
            source: None,
            precision: None,
            pricing_version: None,
            line_items: Vec::new(),
            reason: Some(reason.into()),
        });
    }

    #[must_use]
    pub fn limit_hit(&self) -> bool {
        self.max_cost
            .as_ref()
            .is_some_and(|limit| self.known_total >= limit.amount)
    }

    #[must_use]
    pub fn report(&self) -> RunCostReport {
        let status = if self.requests.is_empty() || self.priced_requests == 0 {
            CostReportStatus::Unavailable
        } else if self.unpriced_requests == 0 {
            CostReportStatus::Complete
        } else {
            CostReportStatus::Partial
        };
        let known_total = (self.priced_requests > 0).then(|| {
            Money::new(self.known_total, self.target_currency.clone()).expect("validated")
        });
        let budget_status = if self.max_cost.is_none() {
            "not_configured"
        } else if self.unpriced_requests > 0 {
            "degraded"
        } else if self.limit_hit() {
            "limit_reached"
        } else {
            "enforced"
        };
        RunCostReport {
            status,
            known_total,
            priced_requests: self.priced_requests,
            unpriced_requests: self.unpriced_requests,
            budget_status: budget_status.to_string(),
            requests: self.requests.clone(),
        }
    }
}

fn serialize_decimal<S>(value: &Decimal, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

fn serialize_decimal_map<S>(
    value: &BTreeMap<String, Decimal>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    use serde::ser::SerializeMap;

    let mut map = serializer.serialize_map(Some(value.len()))?;
    for (key, amount) in value {
        map.serialize_entry(key, &amount.to_string())?;
    }
    map.end()
}

fn deserialize_decimal<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    decimal_from_value(&value).map_err(serde::de::Error::custom)
}

fn deserialize_optional_decimal<'de, D>(deserializer: D) -> Result<Option<Decimal>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    value
        .as_ref()
        .map(decimal_from_value)
        .transpose()
        .map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt() -> ProviderAttemptAccounting {
        ProviderAttemptAccounting {
            attempt: 1,
            provider_id: "demo".to_string(),
            requested_model: "cheap".to_string(),
            resolved_model: None,
            service_tier: None,
            request_id: Some("req-1".to_string()),
            http_status: Some(200),
            occurred_at_ms: 100,
            usage_reported: true,
            usage: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 2,
                total_tokens: 12,
            },
            billable_metrics: BTreeMap::from([
                (BillableMetric::InputTokens, 10),
                (BillableMetric::OutputTokens, 2),
            ]),
            reported_cost: None,
            error: None,
        }
    }

    #[test]
    fn tiny_decimal_is_exact() {
        let parsed = parse_decimal("0.00000894").expect("decimal");
        let total = parsed.checked_add(parsed).expect("sum");
        assert_eq!(total.to_string(), "0.00001788");
    }

    #[test]
    fn json_scientific_decimal_is_exact() {
        let value: Value = serde_json::from_str("8.94e-6").expect("json");
        assert_eq!(
            decimal_from_value(&value).expect("decimal").to_string(),
            "0.00000894"
        );
    }

    #[tokio::test]
    async fn catalog_prices_exact_token_rates() {
        let catalog: PricingCatalog = serde_yaml::from_str(
            r#"
version: "2026-08-06"
source: "fixture"
rules:
  - provider_id: demo
    model: cheap
    service_tier: null
    currency: USD
    rates:
      input_tokens: { amount: "0.10", per: 1000000 }
      output_tokens: { amount: "0.20", per: 1000000 }
"#,
        )
        .expect("catalog");
        let resolver = CatalogCostResolver::new(catalog, "USD").expect("resolver");
        let CostResolution::Priced { money, .. } = resolver.resolve(&attempt()).await else {
            panic!("expected priced");
        };
        assert_eq!(money.amount.to_string(), "0.0000014");
    }

    #[tokio::test]
    async fn unavailable_source_falls_back_to_catalog() {
        let catalog: PricingCatalog = serde_yaml::from_str(
            r#"
version: "v1"
source: "fixture"
rules:
  - provider_id: demo
    model: cheap
    service_tier: null
    currency: USD
    fixed_cost: "0.00000894"
"#,
        )
        .expect("catalog");
        let chain = CostResolverChain::new(vec![
            Arc::new(UnavailableCostResolver::new("mcp", "offline")),
            Arc::new(CatalogCostResolver::new(catalog, "USD").expect("resolver")),
        ]);
        let CostResolution::Priced { money, source, .. } = chain.resolve(&attempt()).await else {
            panic!("expected fallback price");
        };
        assert_eq!(source, "catalog:fixture");
        assert_eq!(money.amount.to_string(), "0.00000894");
    }

    #[test]
    fn money_serializes_amount_as_string() {
        let money = Money::parse("0.00000894", "USD").expect("money");
        let value = serde_json::to_value(money).expect("json");
        assert_eq!(value["amount"], "0.00000894");
    }

    #[test]
    fn money_deserializes_yaml_number_exactly() {
        let money: Money =
            serde_yaml::from_str("{ amount: 0.00000894, currency: USD }").expect("money");
        assert_eq!(money.amount.to_string(), "0.00000894");
    }

    #[test]
    fn partial_report_keeps_known_total_and_degrades_budget() {
        let mut tracker =
            RunCostTracker::new("USD", Some(Money::parse("1.00", "USD").expect("limit")))
                .expect("tracker");
        tracker.record(
            1,
            &attempt(),
            CostResolution::Priced {
                money: Money::parse("0.00000894", "USD").expect("money"),
                source: "fixture".to_string(),
                precision: CostPrecision::Actual,
                pricing_version: None,
                line_items: Vec::new(),
            },
        );
        tracker.record(
            2,
            &attempt(),
            CostResolution::Unpriced {
                reason: "unknown model".to_string(),
            },
        );

        let report = tracker.report();
        assert_eq!(report.status, CostReportStatus::Partial);
        assert_eq!(report.budget_status, "degraded");
        assert_eq!(
            report.known_total.expect("known").amount.to_string(),
            "0.00000894"
        );
    }
}
