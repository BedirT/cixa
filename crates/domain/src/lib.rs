//! The provider-neutral, deterministic security core for agent-treasury.
//!
//! The core deliberately has no browser, network, MCP, or framework dependency. A
//! caller supplies an authenticated capability token and a typed request. All
//! authorization decisions happen here, outside the untrusted agent.

use chrono::Utc;
use hmac::{Hmac, Mac};
use idna::domain_to_ascii;
use rand::RngCore;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::IpAddr;
use std::path::Path;
use std::sync::LazyLock;
use thiserror::Error;
use url::Url;

pub const API_VERSION: &str = "v1";
pub const STATE_FILE: &str = "state.json";
pub const AUDIT_KEY_FILE: &str = "audit.key";
pub const LOCK_FILE: &str = "treasury.lock";

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Error)]
pub enum TreasuryError {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("money error: {0}")]
    Money(String),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, TreasuryError>;

fn now() -> i64 {
    Utc::now().timestamp()
}

fn random_hex(bytes: usize) -> String {
    let mut value = vec![0_u8; bytes];
    rand::rng().fill_bytes(&mut value);
    hex::encode(value)
}

fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", random_hex(12))
}

fn new_token() -> String {
    random_hex(32)
}

fn token_hash(token: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"agent-treasury capability token v1\0");
    digest.update(token.as_bytes());
    hex::encode(digest.finalize())
}

fn hmac_hash(key: &[u8], value: &Value) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts every non-empty key");
    mac.update(value.to_string().as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn hmac_bytes(key: &[u8], value: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts every non-empty key");
    mac.update(value);
    hex::encode(mac.finalize().into_bytes())
}

fn bounded(value: &str, field: &str, max: usize) -> Result<()> {
    if value.is_empty() || value.chars().count() > max {
        return Err(TreasuryError::Invalid(format!("{field} must contain 1..{max} characters")));
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(TreasuryError::Invalid(format!("{field} contains a control character")));
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Money {
    pub minor: i64,
    pub currency: String,
}

impl Money {
    pub fn new(minor: i64, currency: &str) -> Result<Self> {
        if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err(TreasuryError::Money(
                "currency must be an uppercase ISO 4217 code".to_string(),
            ));
        }
        Ok(Self { minor, currency: currency.to_string() })
    }

    pub fn positive(minor: i64, currency: &str) -> Result<Self> {
        let money = Self::new(minor, currency)?;
        if money.minor <= 0 {
            return Err(TreasuryError::Money("amount must be positive".to_string()));
        }
        Ok(money)
    }

    pub fn zero(currency: &str) -> Result<Self> {
        Self::new(0, currency)
    }

    pub fn checked_add(&self, other: &Self) -> Result<Self> {
        self.same_currency(other)?;
        let minor = self
            .minor
            .checked_add(other.minor)
            .ok_or_else(|| TreasuryError::Money("minor-unit overflow".to_string()))?;
        Ok(Self { minor, currency: self.currency.clone() })
    }

    pub fn checked_sub(&self, other: &Self) -> Result<Self> {
        self.same_currency(other)?;
        let minor = self
            .minor
            .checked_sub(other.minor)
            .ok_or_else(|| TreasuryError::Money("minor-unit underflow".to_string()))?;
        Ok(Self { minor, currency: self.currency.clone() })
    }

    pub fn scaled_basis_points(&self, basis_points: u16) -> Result<Self> {
        if basis_points > 10_000 {
            return Err(TreasuryError::Money(
                "basis points must be between 0 and 10000".to_string(),
            ));
        }
        let scaled =
            i128::from(self.minor).checked_mul(i128::from(basis_points)).ok_or_else(|| {
                TreasuryError::Money("basis-point multiplication overflow".to_string())
            })? / 10_000;
        let minor = i64::try_from(scaled)
            .map_err(|_| TreasuryError::Money("scaled minor-unit overflow".to_string()))?;
        Ok(Self { minor, currency: self.currency.clone() })
    }

    fn same_currency(&self, other: &Self) -> Result<()> {
        if self.currency != other.currency {
            return Err(TreasuryError::Money(format!(
                "currency mismatch: {} and {}",
                self.currency, other.currency
            )));
        }
        Ok(())
    }
}

fn remaining_money(limit: &Money, used: &Money) -> Result<Money> {
    limit.same_currency(used)?;
    Money::new(limit.minor.saturating_sub(used.minor).max(0), &limit.currency)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutonomyMode {
    Observe,
    ApprovalRequired,
    BoundedAutonomous,
    Disabled,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PaymentFormTrust {
    HostedFields,
    OwnerApprovedMerchant,
    MerchantControlled,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SimulatedScenario {
    Normal,
    Decline,
    DelayedSettlement,
    TimeoutBeforeSubmit,
    TimeoutAfterSubmit,
    DuplicateFormSubmission,
    MisleadingSuccessPage,
    PromptInjection,
    AmountChanged,
    CurrencyChanged,
    HiddenRecurring,
    CardSaving,
    Tip,
    Preauthorization,
    MerchantControlledForm,
    RedirectToOtherDomain,
    RedirectToLocalhost,
    DnsRebindingLike,
    BrowserCrash,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub id: String,
    pub version: u64,
    pub primary_currency: String,
    pub max_per_transaction: Money,
    pub max_per_session: Money,
    pub max_rolling_24h: Money,
    pub max_lifetime: Money,
    pub absolute_exposure_ceiling: Money,
    pub max_treasury_size: Money,
    pub reinvestment_ratio_bps: u16,
    pub allowed_currencies: BTreeSet<String>,
    pub allowed_merchants: BTreeSet<String>,
    pub denied_merchants: BTreeSet<String>,
    pub approved_redirect_domains: BTreeSet<String>,
    pub require_approval_for_new_merchants: bool,
    pub approved_fulfillment_profiles: BTreeSet<String>,
    pub allow_recurring: bool,
    pub allow_trials: bool,
    pub allow_stored_card: bool,
    pub allow_tips: bool,
    pub allow_preauthorization: bool,
    pub allow_installments: bool,
    pub denied_categories: BTreeSet<String>,
    pub max_order_total_drift_minor: i64,
    pub max_attempts: u32,
    pub max_transactions_per_minute: u32,
    pub max_redirects: usize,
    pub intent_ttl_secs: i64,
    pub card_session_ttl_secs: i64,
}

impl Policy {
    pub fn conservative_demo() -> Result<Self> {
        let mut allowed_currencies = BTreeSet::new();
        allowed_currencies.insert("CAD".to_string());
        let mut allowed_merchants = BTreeSet::new();
        allowed_merchants.insert("merchant.example.test".to_string());
        let mut fulfillment = BTreeSet::new();
        fulfillment.insert("digital-email".to_string());
        Ok(Self {
            id: "policy_demo".to_string(),
            version: 1,
            primary_currency: "CAD".to_string(),
            max_per_transaction: Money::positive(2_500, "CAD")?,
            max_per_session: Money::positive(5_000, "CAD")?,
            max_rolling_24h: Money::positive(10_000, "CAD")?,
            max_lifetime: Money::positive(25_000, "CAD")?,
            absolute_exposure_ceiling: Money::positive(100_000, "CAD")?,
            max_treasury_size: Money::positive(100_000, "CAD")?,
            reinvestment_ratio_bps: 0,
            allowed_currencies,
            allowed_merchants,
            denied_merchants: BTreeSet::new(),
            approved_redirect_domains: BTreeSet::new(),
            require_approval_for_new_merchants: true,
            approved_fulfillment_profiles: fulfillment,
            allow_recurring: false,
            allow_trials: false,
            allow_stored_card: false,
            allow_tips: false,
            allow_preauthorization: false,
            allow_installments: false,
            denied_categories: [
                "gambling",
                "crypto",
                "financial_transfer",
                "cash_withdrawal",
                "gift_card",
                "cash_equivalent",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            max_order_total_drift_minor: 0,
            max_attempts: 1,
            max_transactions_per_minute: 10,
            max_redirects: 2,
            intent_ttl_secs: 15 * 60,
            card_session_ttl_secs: 10 * 60,
        })
    }

    pub fn validate(&self) -> Result<()> {
        Money::zero(&self.primary_currency)?;
        if self.reinvestment_ratio_bps > 10_000 {
            return Err(TreasuryError::Invalid(
                "reinvestment ratio must be 0..10000 basis points".to_string(),
            ));
        }
        if self.max_transactions_per_minute == 0 {
            return Err(TreasuryError::Invalid(
                "transaction rate limit must be positive".to_string(),
            ));
        }
        if self.intent_ttl_secs <= 0
            || self.card_session_ttl_secs <= 0
            || self.max_order_total_drift_minor < 0
        {
            return Err(TreasuryError::Invalid(
                "policy expiry and total-drift values are invalid".to_string(),
            ));
        }
        for value in [
            &self.max_per_transaction,
            &self.max_per_session,
            &self.max_rolling_24h,
            &self.max_lifetime,
            &self.absolute_exposure_ceiling,
            &self.max_treasury_size,
        ] {
            if value.currency != self.primary_currency || value.minor <= 0 {
                return Err(TreasuryError::Invalid(
                    "policy money values must be positive and use the primary currency".to_string(),
                ));
            }
        }
        if self.absolute_exposure_ceiling.minor > self.max_treasury_size.minor {
            return Err(TreasuryError::Invalid(
                "exposure ceiling cannot exceed maximum treasury size".to_string(),
            ));
        }
        for merchant in self
            .allowed_merchants
            .iter()
            .chain(self.denied_merchants.iter())
            .chain(self.approved_redirect_domains.iter())
        {
            if canonicalize_domain(merchant)? != *merchant {
                return Err(TreasuryError::Invalid(
                    "policy merchant domains must be canonical".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn evaluate(
        &self,
        request: &PurchaseRequest,
        usage: &BudgetUsage,
        mode: &AutonomyMode,
        known_merchant: bool,
        emergency_stop: bool,
        at: i64,
    ) -> Result<PolicyDecision> {
        self.validate()?;
        let mut reasons = Vec::new();
        let mut approval = false;
        request.validate(self, at)?;

        if emergency_stop {
            reasons.push("emergency stop is active".to_string());
        }
        if matches!(mode, AutonomyMode::Observe | AutonomyMode::Disabled) {
            reasons.push("agent is not armed for spending".to_string());
        }
        if !self.allowed_currencies.contains(&request.amount.currency) {
            reasons.push("currency is not allowed".to_string());
        }
        if request.amount.currency != self.primary_currency {
            reasons.push("foreign exchange is denied by default".to_string());
        }
        if request.amount.currency == self.primary_currency {
            if request.amount.minor > self.max_per_transaction.minor {
                reasons.push("per-transaction limit exceeded".to_string());
            }
            if usage.session_amount.checked_add(&request.amount)?.minor > self.max_per_session.minor
            {
                reasons.push("per-session limit exceeded".to_string());
            }
            if usage.rolling_24h_amount.checked_add(&request.amount)?.minor
                > self.max_rolling_24h.minor
            {
                reasons.push("rolling 24-hour limit exceeded".to_string());
            }
            if usage.lifetime_amount.checked_add(&request.amount)?.minor > self.max_lifetime.minor {
                reasons.push("lifetime limit exceeded".to_string());
            }
            if usage.recent_transaction_count >= self.max_transactions_per_minute {
                reasons.push("transaction rate limit exceeded".to_string());
            }
        }
        let merchant = canonicalize_domain(&request.merchant_domain)?;
        if self.denied_merchants.contains(&merchant) {
            reasons.push("merchant is explicitly denied".to_string());
        }
        if !self.allowed_merchants.is_empty() && !self.allowed_merchants.contains(&merchant) {
            if self.require_approval_for_new_merchants {
                approval = true;
            } else {
                reasons.push("merchant is not allowlisted".to_string());
            }
        }
        if !known_merchant
            && !self.allowed_merchants.contains(&merchant)
            && self.require_approval_for_new_merchants
        {
            approval = true;
        }
        if !self.approved_fulfillment_profiles.contains(&request.fulfillment_profile) {
            reasons.push("fulfillment profile is not approved".to_string());
        }
        if (request.recurring || request.trial_auto_renew)
            && (!self.allow_recurring || request.trial_auto_renew && !self.allow_trials)
        {
            reasons.push("recurring or auto-renewing purchase is denied".to_string());
        }
        if request.stored_card && !self.allow_stored_card {
            reasons.push("stored-card consent is denied".to_string());
        }
        if request.tip_minor > 0 && !self.allow_tips {
            reasons.push("tips and open-ended totals are denied".to_string());
        }
        if request.preauthorization && !self.allow_preauthorization {
            reasons.push("preauthorizations are denied".to_string());
        }
        if request.installments && !self.allow_installments {
            reasons.push("installments and buy-now-pay-later are denied".to_string());
        }
        if self.denied_categories.contains(&request.category) {
            reasons.push(format!("category {} is denied", request.category));
        }
        if request.redirect_chain.len() > self.max_redirects {
            reasons.push("redirect limit exceeded".to_string());
        }
        for redirect in &request.redirect_chain {
            let redirect_host = validate_https_url(redirect)?;
            if redirect_host != merchant && !self.approved_redirect_domains.contains(&redirect_host)
            {
                reasons.push(format!("redirect domain {redirect_host} is not owner-approved"));
            }
        }
        if request.payment_form == PaymentFormTrust::MerchantControlled {
            approval = true;
        }
        if request.scenario == SimulatedScenario::PromptInjection {
            approval = true;
        }
        if request.scenario == SimulatedScenario::AmountChanged
            || request.scenario == SimulatedScenario::CurrencyChanged
            || request.scenario == SimulatedScenario::HiddenRecurring
            || request.scenario == SimulatedScenario::CardSaving
            || request.scenario == SimulatedScenario::Tip
            || request.scenario == SimulatedScenario::Preauthorization
            || request.scenario == SimulatedScenario::RedirectToLocalhost
            || request.scenario == SimulatedScenario::DnsRebindingLike
        {
            approval = true;
        }
        if !known_merchant && matches!(mode, AutonomyMode::ApprovalRequired) {
            approval = true;
        }
        if matches!(mode, AutonomyMode::ApprovalRequired) {
            approval = true;
        }
        if !reasons.is_empty() {
            return Ok(PolicyDecision {
                allowed: false,
                requires_approval: false,
                reasons,
                policy_version: self.version,
            });
        }
        Ok(PolicyDecision {
            allowed: !approval,
            requires_approval: approval,
            reasons: if approval {
                vec!["owner approval is required".to_string()]
            } else {
                Vec::new()
            },
            policy_version: self.version,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PurchaseRequest {
    pub idempotency_key: String,
    pub amount: Money,
    pub final_total: Money,
    pub merchant_domain: String,
    pub category: String,
    pub recurring: bool,
    pub trial_auto_renew: bool,
    pub stored_card: bool,
    pub tip_minor: i64,
    pub preauthorization: bool,
    pub installments: bool,
    pub fulfillment_profile: String,
    pub payment_form: PaymentFormTrust,
    pub redirect_chain: Vec<String>,
    pub attempts: u32,
    pub session_id: String,
    pub scenario: SimulatedScenario,
}

impl PurchaseRequest {
    fn validate(&self, policy: &Policy, at: i64) -> Result<()> {
        bounded(&self.idempotency_key, "idempotency_key", 128)?;
        bounded(&self.merchant_domain, "merchant_domain", 253)?;
        bounded(&self.category, "category", 64)?;
        bounded(&self.fulfillment_profile, "fulfillment_profile", 64)?;
        bounded(&self.session_id, "session_id", 128)?;
        if self.amount.minor <= 0 || self.final_total.minor <= 0 {
            return Err(TreasuryError::Invalid("purchase amount must be positive".to_string()));
        }
        if self.amount.currency != self.final_total.currency {
            return Err(TreasuryError::Invalid(
                "final total currency must match intent currency".to_string(),
            ));
        }
        let allowed_total = self
            .amount
            .minor
            .checked_add(policy.max_order_total_drift_minor)
            .ok_or_else(|| TreasuryError::Money("order-total overflow".to_string()))?;
        if self.final_total.minor > allowed_total {
            return Err(TreasuryError::Invalid(
                "final total exceeds the configured drift allowance".to_string(),
            ));
        }
        if self.attempts == 0 || self.attempts > policy.max_attempts {
            return Err(TreasuryError::Invalid("attempt count exceeds policy".to_string()));
        }
        if self.redirect_chain.len() > policy.max_redirects {
            return Err(TreasuryError::Invalid("redirect chain is too long".to_string()));
        }
        if self.idempotency_key.len() > 128 {
            return Err(TreasuryError::Invalid("idempotency key is too long".to_string()));
        }
        let _ = at;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyDecision {
    pub allowed: bool,
    pub requires_approval: bool,
    pub reasons: Vec<String>,
    pub policy_version: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BudgetUsage {
    pub session_amount: Money,
    pub rolling_24h_amount: Money,
    pub lifetime_amount: Money,
    pub reserved_amount: Money,
    pub recent_transaction_count: u32,
}

impl BudgetUsage {
    fn zero(currency: &str) -> Result<Self> {
        let zero = Money::zero(currency)?;
        Ok(Self {
            session_amount: zero.clone(),
            rolling_24h_amount: zero.clone(),
            lifetime_amount: zero.clone(),
            reserved_amount: zero,
            recent_transaction_count: 0,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransactionState {
    Draft,
    Proposed,
    PolicyValidated,
    ApprovalRequired,
    Approved,
    FundsReserved,
    Executing,
    ProviderPending,
    Settled,
    Declined,
    Failed,
    Unknown,
    Cancelled,
    Refunded,
    ReconciliationRequired,
}

impl TransactionState {
    pub fn can_transition(&self, next: &Self) -> bool {
        use TransactionState::*;
        matches!(
            (self, next),
            (Draft, Proposed)
                | (Proposed, PolicyValidated)
                | (Proposed, ApprovalRequired)
                | (Proposed, Failed)
                | (Proposed, Cancelled)
                | (PolicyValidated, FundsReserved)
                | (ApprovalRequired, Approved)
                | (ApprovalRequired, Cancelled)
                | (Approved, FundsReserved)
                | (FundsReserved, Executing)
                | (Executing, ProviderPending)
                | (Executing, Settled)
                | (Executing, Declined)
                | (Executing, Failed)
                | (Executing, Unknown)
                | (ProviderPending, Settled)
                | (ProviderPending, Declined)
                | (ProviderPending, Unknown)
                | (ProviderPending, ReconciliationRequired)
                | (Unknown, ReconciliationRequired)
                | (ReconciliationRequired, Settled)
                | (ReconciliationRequired, Declined)
                | (Unknown, Settled)
                | (Unknown, Declined)
                | (Settled, Refunded)
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PurchaseIntent {
    pub id: String,
    pub agent_id: String,
    pub broker_session_id: String,
    pub request: PurchaseRequest,
    pub state: TransactionState,
    pub decision: PolicyDecision,
    pub policy_version: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub provider_reference: Option<String>,
    pub receipt_hash: Option<String>,
    pub last_error: Option<String>,
}

impl PurchaseIntent {
    fn transition(&mut self, next: TransactionState) -> Result<()> {
        if !self.state.can_transition(&next) {
            return Err(TreasuryError::Conflict(format!(
                "invalid transaction transition {:?} -> {:?}",
                self.state, next
            )));
        }
        self.state = next;
        self.updated_at = now();
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Receipt {
    pub intent_id: String,
    pub merchant_domain: String,
    pub amount: Money,
    pub status: TransactionState,
    pub provider_reference: Option<String>,
    pub issued_at: i64,
    pub personal_information_redacted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LedgerEventKind {
    OwnerCapital,
    IncomeVerified,
    IncomeUnverified,
    OperatorTopUp,
    HoldReserved,
    HoldReleased,
    SpendingSettled,
    Reversal,
    Refund,
    UnknownCharge,
    Reconciliation,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LedgerEvent {
    pub id: String,
    pub at: i64,
    pub kind: LedgerEventKind,
    pub amount: Money,
    pub intent_id: Option<String>,
    pub source: String,
    pub verified: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LedgerSnapshot {
    pub currency: String,
    pub owner_capital: Money,
    pub verified_income: Money,
    pub unverified_income: Money,
    pub operator_topups: Money,
    pub settled_spending: Money,
    pub refunds: Money,
    pub reserved_amount: Money,
    pub available_authority: Money,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReceiveInstructions {
    pub method: String,
    pub address: String,
    pub memo_template: String,
    pub public: bool,
    pub configured_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderCharge {
    pub intent_id: String,
    pub amount: Money,
    pub provider_reference: String,
    pub settled: bool,
    pub refunded: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SimulatedProvider {
    pub provider_id: String,
    pub balance: Money,
    pub holds: BTreeMap<String, Money>,
    pub charges: BTreeMap<String, ProviderCharge>,
    pub incoming_deposits: Vec<Money>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderOutcome {
    Approved { reference: String },
    Declined { reason: String },
    Pending { reference: String },
    Unknown { reason: String },
}

/// Boundary for a future officially authenticated financial adapter. The core
/// never assumes that its ledger is the provider's authoritative balance.
pub trait PaymentProvider {
    fn provider_id(&self) -> &str;
    fn available_balance(&self) -> Result<Money>;
    fn authorize(&mut self, intent: &PurchaseIntent) -> Result<ProviderOutcome>;
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BalanceStatus {
    Estimated,
    OwnerConfirmed,
    ProviderVerified,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManualBalanceSnapshot {
    pub amount: Money,
    pub status: BalanceStatus,
    pub observed_at: i64,
    pub source: String,
    pub expires_at: i64,
}

/// A manual card adapter never logs in to an issuer and cannot submit by
/// itself. It carries only a secret reference and owner-entered freshness data.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManualPrepaidCardProvider {
    pub provider_id: String,
    pub card: SecretReference,
    pub balance_snapshot: Option<ManualBalanceSnapshot>,
    pub outgoing_supported: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManualProviderConfiguration {
    pub credential_reference: String,
    pub provider_kind: String,
    pub last_four: Option<String>,
    pub balance: Money,
    pub balance_status: BalanceStatus,
    pub balance_ttl_secs: i64,
}

impl ManualPrepaidCardProvider {
    pub fn new(card: SecretReference) -> Self {
        Self {
            provider_id: "manual-prepaid-card".to_string(),
            card,
            balance_snapshot: None,
            outgoing_supported: false,
        }
    }

    pub fn set_owner_confirmed_balance(
        &mut self,
        amount: Money,
        source: &str,
        ttl_secs: i64,
    ) -> Result<()> {
        bounded(source, "balance_source", 128)?;
        if ttl_secs <= 0 || amount.minor < 0 {
            return Err(TreasuryError::Invalid("balance snapshot is invalid".to_string()));
        }
        let at = now();
        self.balance_snapshot = Some(ManualBalanceSnapshot {
            amount,
            status: BalanceStatus::OwnerConfirmed,
            observed_at: at,
            source: source.to_string(),
            expires_at: at + ttl_secs,
        });
        Ok(())
    }
}

impl PaymentProvider for ManualPrepaidCardProvider {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn available_balance(&self) -> Result<Money> {
        let snapshot = self.balance_snapshot.as_ref().ok_or_else(|| {
            TreasuryError::Conflict("manual balance requires owner reconciliation".to_string())
        })?;
        if snapshot.expires_at <= now() {
            return Err(TreasuryError::Conflict(
                "manual balance snapshot has expired; owner refresh is required".to_string(),
            ));
        }
        if snapshot.status == BalanceStatus::Estimated {
            return Err(TreasuryError::Conflict(
                "estimated manual balances cannot authorize spending".to_string(),
            ));
        }
        Ok(snapshot.amount.clone())
    }

    fn authorize(&mut self, _intent: &PurchaseIntent) -> Result<ProviderOutcome> {
        Ok(ProviderOutcome::Unknown {
            reason: "manual card provider requires owner-controlled checkout and reconciliation"
                .to_string(),
        })
    }
}

/// A checkout executor is deliberately narrower than a browser driver. The
/// broker must validate the request before an executor may enter the payment
/// critical section.
pub trait CheckoutExecutor {
    fn executor_id(&self) -> &str;
    fn validate_origin_and_total(&self, request: &PurchaseRequest) -> Result<()>;
    fn submit_once(&mut self, intent: &PurchaseIntent) -> Result<ProviderOutcome>;
}

impl SimulatedProvider {
    pub fn new(balance: Money) -> Self {
        Self {
            provider_id: "simulated-local".to_string(),
            balance,
            holds: BTreeMap::new(),
            charges: BTreeMap::new(),
            incoming_deposits: Vec::new(),
        }
    }

    pub fn available_balance(&self) -> Result<Money> {
        let mut reserved = Money::zero(&self.balance.currency)?;
        for value in self.holds.values() {
            reserved = reserved.checked_add(value)?;
        }
        self.balance.checked_sub(&reserved)
    }

    pub fn authorize(&mut self, intent: &PurchaseIntent) -> Result<ProviderOutcome> {
        if let Some(charge) = self.charges.get(&intent.id) {
            return Ok(ProviderOutcome::Approved { reference: charge.provider_reference.clone() });
        }
        match intent.request.scenario {
            SimulatedScenario::Decline => {
                return Ok(ProviderOutcome::Declined {
                    reason: "simulated issuer decline".to_string(),
                });
            }
            SimulatedScenario::TimeoutBeforeSubmit => {
                return Ok(ProviderOutcome::Unknown {
                    reason: "network failed before the broker observed submission".to_string(),
                });
            }
            _ => {}
        }
        if self.available_balance()?.minor < intent.request.amount.minor {
            return Ok(ProviderOutcome::Declined {
                reason: "simulated provider balance is insufficient".to_string(),
            });
        }
        let reference = new_id("sim_charge");
        if intent.request.scenario == SimulatedScenario::DelayedSettlement {
            self.holds.insert(intent.id.clone(), intent.request.amount.clone());
            return Ok(ProviderOutcome::Pending { reference });
        }
        self.balance = self.balance.checked_sub(&intent.request.amount)?;
        self.charges.insert(
            intent.id.clone(),
            ProviderCharge {
                intent_id: intent.id.clone(),
                amount: intent.request.amount.clone(),
                provider_reference: reference.clone(),
                settled: intent.request.scenario != SimulatedScenario::TimeoutAfterSubmit,
                refunded: false,
            },
        );
        if intent.request.scenario == SimulatedScenario::TimeoutAfterSubmit
            || intent.request.scenario == SimulatedScenario::BrowserCrash
        {
            return Ok(ProviderOutcome::Unknown {
                reason: "provider may have accepted the charge but the result was lost".to_string(),
            });
        }
        Ok(ProviderOutcome::Approved { reference })
    }

    pub fn settle_pending(&mut self, intent: &PurchaseIntent) -> Result<String> {
        let amount = self
            .holds
            .remove(&intent.id)
            .ok_or_else(|| TreasuryError::NotFound("provider hold".to_string()))?;
        self.balance = self.balance.checked_sub(&amount)?;
        let reference = new_id("sim_charge");
        self.charges.insert(
            intent.id.clone(),
            ProviderCharge {
                intent_id: intent.id.clone(),
                amount,
                provider_reference: reference.clone(),
                settled: true,
                refunded: false,
            },
        );
        Ok(reference)
    }

    fn refund(&mut self, intent_id: &str) -> Result<Money> {
        let charge = self
            .charges
            .get_mut(intent_id)
            .ok_or_else(|| TreasuryError::NotFound("provider charge".to_string()))?;
        if charge.refunded {
            return Err(TreasuryError::Conflict(
                "provider charge was already refunded".to_string(),
            ));
        }
        charge.refunded = true;
        self.balance = self.balance.checked_add(&charge.amount)?;
        Ok(charge.amount.clone())
    }

    fn release_hold(&mut self, intent_id: &str) -> Option<Money> {
        self.holds.remove(intent_id)
    }

    fn reconcile_settled(&mut self, intent: &PurchaseIntent, reference: &str) -> Result<String> {
        if self.holds.contains_key(&intent.id) {
            return self.settle_pending(intent);
        }
        if let Some(charge) = self.charges.get_mut(&intent.id) {
            charge.settled = true;
            return Ok(charge.provider_reference.clone());
        }
        if self.available_balance()?.minor < intent.request.amount.minor {
            return Err(TreasuryError::Money(
                "provider balance is insufficient for reconciled settlement".to_string(),
            ));
        }
        self.balance = self.balance.checked_sub(&intent.request.amount)?;
        self.charges.insert(
            intent.id.clone(),
            ProviderCharge {
                intent_id: intent.id.clone(),
                amount: intent.request.amount.clone(),
                provider_reference: reference.to_string(),
                settled: true,
                refunded: false,
            },
        );
        Ok(reference.to_string())
    }
}

impl PaymentProvider for SimulatedProvider {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn available_balance(&self) -> Result<Money> {
        self.available_balance()
    }

    fn authorize(&mut self, intent: &PurchaseIntent) -> Result<ProviderOutcome> {
        self.authorize(intent)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentRecord {
    pub id: String,
    pub name: String,
    pub capability_token_hash: String,
    pub scopes: BTreeSet<String>,
    pub policy_id: String,
    pub approved_merchants: BTreeSet<String>,
    pub broker_session_id: String,
    pub broker_session_expires_at: i64,
    pub mode: AutonomyMode,
    pub created_at: i64,
    pub expires_at: i64,
    pub revoked: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OwnerRecord {
    pub id: String,
    pub name: String,
    pub capability_token_hash: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditEntry {
    pub sequence: u64,
    pub at: i64,
    pub actor: String,
    pub action: String,
    pub intent_id: Option<String>,
    pub policy_version: Option<u64>,
    pub decision: Option<String>,
    pub details: Value,
    pub previous_hash: String,
    pub hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryState {
    pub schema_version: u32,
    pub generation: u64,
    pub owner: OwnerRecord,
    pub agents: BTreeMap<String, AgentRecord>,
    pub policies: BTreeMap<String, Policy>,
    pub intents: BTreeMap<String, PurchaseIntent>,
    pub ledger: Vec<LedgerEvent>,
    pub audit: Vec<AuditEntry>,
    pub receive_instructions: Option<ReceiveInstructions>,
    pub emergency_stop: bool,
    pub provider: SimulatedProvider,
    pub provider_mode: ProviderMode,
    pub manual_provider: Option<ManualPrepaidCardProvider>,
    pub processed_deposits: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderMode {
    Simulated,
    ManualPrepaidCard,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct PersistedEnvelope {
    state: TreasuryState,
    state_mac: String,
}

#[derive(Clone, Debug)]
pub struct Treasury {
    pub state: TreasuryState,
    audit_key: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Bootstrap {
    pub treasury: Treasury,
    pub owner_token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    GetStatus,
    GetCapabilities,
    GetBudget,
    GetReceiveInstructions,
    CreatePurchaseIntent {
        request: PurchaseRequest,
    },
    GetPurchaseIntent {
        intent_id: String,
    },
    ExecutePurchaseIntent {
        intent_id: String,
    },
    CancelPurchaseIntent {
        intent_id: String,
    },
    ListTransactions,
    GetReceipt {
        intent_id: String,
    },
    OwnerCreateAgent {
        name: String,
        policy: Policy,
        mode: AutonomyMode,
        ttl_secs: i64,
    },
    OwnerUpdatePolicy {
        agent_id: String,
        policy: Policy,
    },
    OwnerSetAgentMode {
        agent_id: String,
        mode: AutonomyMode,
    },
    OwnerRevokeAgent {
        agent_id: String,
    },
    OwnerSetEmergencyStop {
        stopped: bool,
    },
    OwnerApproveIntent {
        intent_id: String,
    },
    OwnerReconcile {
        intent_id: String,
        outcome: ReconciliationOutcome,
        provider_reference: Option<String>,
    },
    OwnerRecordDeposit {
        amount: Money,
        source: String,
        verified: bool,
        agent_id: Option<String>,
        external_reference: String,
    },
    OwnerArmAgentSession {
        agent_id: String,
        ttl_secs: i64,
    },
    OwnerConfigureManualProvider {
        credential_reference: String,
        provider_kind: String,
        last_four: Option<String>,
        balance: Money,
        balance_status: BalanceStatus,
        balance_ttl_secs: i64,
    },
    OwnerConfigureReceiveInstructions {
        method: String,
        address: String,
        memo_template: String,
    },
    OwnerListAudit,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationOutcome {
    Settled,
    Declined,
    Refunded,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RpcRequest {
    pub api_version: String,
    pub request_id: String,
    pub token: String,
    pub operation: Request,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RpcResponse {
    pub api_version: String,
    pub request_id: String,
    pub ok: bool,
    pub data: Option<Value>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Actor {
    Owner,
    Agent(String),
}

impl Treasury {
    pub fn bootstrap(owner_name: &str, initial_balance: Money) -> Result<Bootstrap> {
        bounded(owner_name, "owner_name", 128)?;
        let owner_token = new_token();
        let policy = Policy::conservative_demo()?;
        if initial_balance.currency != policy.primary_currency
            || initial_balance.minor <= 0
            || initial_balance.minor > policy.max_treasury_size.minor
        {
            return Err(TreasuryError::Invalid(
                "initial balance exceeds the conservative treasury policy".to_string(),
            ));
        }
        let owner = OwnerRecord {
            id: new_id("owner"),
            name: owner_name.to_string(),
            capability_token_hash: token_hash(&owner_token),
            created_at: now(),
        };
        let mut treasury = Self {
            state: TreasuryState {
                schema_version: 2,
                generation: 0,
                owner,
                agents: BTreeMap::new(),
                policies: BTreeMap::from([(policy.id.clone(), policy)]),
                intents: BTreeMap::new(),
                ledger: Vec::new(),
                audit: Vec::new(),
                receive_instructions: None,
                emergency_stop: false,
                provider: SimulatedProvider::new(initial_balance.clone()),
                provider_mode: ProviderMode::Simulated,
                manual_provider: None,
                processed_deposits: BTreeMap::new(),
            },
            audit_key: hex::decode(random_hex(32)).expect("generated hex is valid"),
        };
        treasury.append_ledger(
            LedgerEventKind::OwnerCapital,
            initial_balance,
            None,
            "bootstrap",
            true,
        )?;
        treasury.audit(
            "owner",
            "bootstrap",
            None,
            None,
            Some("allowed"),
            json!({ "provider": "simulated-local" }),
        )?;
        Ok(Bootstrap { treasury, owner_token })
    }

    pub fn load_from(data_dir: &Path) -> Result<Self> {
        let key = read_private_file(&data_dir.join(AUDIT_KEY_FILE))?;
        if key.len() < 32 {
            return Err(TreasuryError::Invalid("audit key is too short".to_string()));
        }
        let encoded = read_private_file(&data_dir.join(STATE_FILE))?;
        let envelope: PersistedEnvelope = serde_json::from_slice(&encoded)?;
        let state_bytes = serde_json::to_vec(&envelope.state)?;
        let expected_mac = hmac_bytes(&key, &state_bytes);
        if !constant_time_eq(expected_mac.as_bytes(), envelope.state_mac.as_bytes()) {
            return Err(TreasuryError::Conflict(
                "persisted treasury state failed integrity verification".to_string(),
            ));
        }
        let state = envelope.state;
        let treasury = Self { state, audit_key: key };
        treasury.validate_state()?;
        treasury.verify_audit_chain()?;
        Ok(treasury)
    }

    pub fn save_to(&self, data_dir: &Path) -> Result<()> {
        ensure_private_directory(data_dir)?;
        let key_path = data_dir.join(AUDIT_KEY_FILE);
        if key_path.exists() {
            let persisted_key = read_private_file(&key_path)?;
            if !constant_time_eq(&persisted_key, &self.audit_key) {
                return Err(TreasuryError::Conflict(
                    "audit key does not match the in-memory treasury".to_string(),
                ));
            }
        } else {
            write_new_private_file(&key_path, &self.audit_key)?;
            sync_directory(data_dir)?;
        }
        self.validate_state()?;
        self.verify_audit_chain()?;
        let state_bytes = serde_json::to_vec(&self.state)?;
        let envelope = PersistedEnvelope {
            state: self.state.clone(),
            state_mac: hmac_bytes(&self.audit_key, &state_bytes),
        };
        let encoded = serde_json::to_vec_pretty(&envelope)?;
        write_atomic_private_file(&data_dir.join(STATE_FILE), &encoded)?;
        sync_directory(data_dir)?;
        Ok(())
    }

    pub fn verify_audit_chain(&self) -> Result<()> {
        let mut previous = String::new();
        for (index, entry) in self.state.audit.iter().enumerate() {
            let expected_sequence = index as u64 + 1;
            if entry.sequence != expected_sequence {
                return Err(TreasuryError::Conflict(format!(
                    "audit sequence {} is not contiguous",
                    entry.sequence
                )));
            }
            if entry.previous_hash != previous {
                return Err(TreasuryError::Conflict(format!(
                    "audit sequence {} has a broken predecessor",
                    entry.sequence
                )));
            }
            let unsigned = json!({
                "sequence": entry.sequence,
                "at": entry.at,
                "actor": entry.actor,
                "action": entry.action,
                "intent_id": entry.intent_id,
                "policy_version": entry.policy_version,
                "decision": entry.decision,
                "details": entry.details,
                "previous_hash": entry.previous_hash,
            });
            let expected = hmac_hash(&self.audit_key, &unsigned);
            if expected != entry.hash {
                return Err(TreasuryError::Conflict(format!(
                    "audit sequence {} has an invalid hash",
                    entry.sequence
                )));
            }
            previous = entry.hash.clone();
        }
        Ok(())
    }

    pub fn recover_interrupted_executions(&mut self) -> Result<usize> {
        let ids: Vec<String> = self
            .state
            .intents
            .values()
            .filter(|intent| intent.state == TransactionState::Executing)
            .map(|intent| intent.id.clone())
            .collect();
        for id in &ids {
            let mut intent = self.state.intents.get(id).expect("collected intent exists").clone();
            intent.transition(TransactionState::Unknown)?;
            intent.last_error = Some(
                "broker restarted during provider submission; automatic retry is forbidden"
                    .to_string(),
            );
            self.state.intents.insert(id.clone(), intent.clone());
            self.audit(
                "broker",
                "recover_interrupted_execution",
                Some(id),
                Some(intent.policy_version),
                Some("unknown"),
                json!({ "retry": false }),
            )?;
        }
        Ok(ids.len())
    }

    fn validate_state(&self) -> Result<()> {
        if self.state.schema_version != 2 {
            return Err(TreasuryError::Invalid("unsupported persisted state schema".to_string()));
        }
        if self.state.owner.capability_token_hash.len() != 64 {
            return Err(TreasuryError::Invalid("owner token hash is invalid".to_string()));
        }
        for (id, policy) in &self.state.policies {
            if id != &policy.id {
                return Err(TreasuryError::Invalid(
                    "policy map key does not match policy id".to_string(),
                ));
            }
            policy.validate()?;
        }
        for (id, agent) in &self.state.agents {
            if id != &agent.id
                || agent.capability_token_hash.len() != 64
                || !self.state.policies.contains_key(&agent.policy_id)
                || agent.broker_session_id.is_empty()
                || agent.broker_session_expires_at > agent.expires_at
            {
                return Err(TreasuryError::Invalid("persisted agent invariant failed".to_string()));
            }
            for merchant in &agent.approved_merchants {
                if canonicalize_domain(merchant)? != *merchant {
                    return Err(TreasuryError::Invalid(
                        "persisted approved merchant is not canonical".to_string(),
                    ));
                }
            }
        }
        for (id, intent) in &self.state.intents {
            if id != &intent.id
                || !self.state.agents.contains_key(&intent.agent_id)
                || intent.broker_session_id.is_empty()
            {
                return Err(TreasuryError::Invalid(
                    "persisted intent invariant failed".to_string(),
                ));
            }
            Money::positive(intent.request.amount.minor, &intent.request.amount.currency)?;
            Money::positive(
                intent.request.final_total.minor,
                &intent.request.final_total.currency,
            )?;
        }
        for event in &self.state.ledger {
            Money::new(event.amount.minor, &event.amount.currency)?;
            if let Some(intent_id) = &event.intent_id
                && !self.state.intents.contains_key(intent_id)
            {
                return Err(TreasuryError::Invalid(
                    "ledger event refers to an unknown intent".to_string(),
                ));
            }
        }
        match self.state.provider_mode {
            ProviderMode::Simulated => {
                if self.state.provider.balance.minor < 0 {
                    return Err(TreasuryError::Invalid(
                        "simulated provider balance cannot be negative".to_string(),
                    ));
                }
            }
            ProviderMode::ManualPrepaidCard => {
                let provider = self.state.manual_provider.as_ref().ok_or_else(|| {
                    TreasuryError::Invalid(
                        "manual provider mode requires manual provider configuration".to_string(),
                    )
                })?;
                if provider.card.persisted_secret || provider.card.reference.is_empty() {
                    return Err(TreasuryError::Invalid(
                        "manual provider must persist only a non-secret credential reference"
                            .to_string(),
                    ));
                }
            }
        }
        for reference in self.state.processed_deposits.keys() {
            bounded(reference, "external_reference", 160)?;
        }
        Ok(())
    }

    fn provider_id(&self) -> &str {
        match self.state.provider_mode {
            ProviderMode::Simulated => &self.state.provider.provider_id,
            ProviderMode::ManualPrepaidCard => self
                .state
                .manual_provider
                .as_ref()
                .map(|provider| provider.provider_id.as_str())
                .unwrap_or("manual-provider-unconfigured"),
        }
    }

    fn provider_available_balance(&self) -> Result<Money> {
        match self.state.provider_mode {
            ProviderMode::Simulated => self.state.provider.available_balance(),
            ProviderMode::ManualPrepaidCard => self
                .state
                .manual_provider
                .as_ref()
                .ok_or_else(|| {
                    TreasuryError::Conflict("manual provider is not configured".to_string())
                })?
                .available_balance(),
        }
    }

    fn provider_reported_balance(&self) -> Result<Money> {
        match self.state.provider_mode {
            ProviderMode::Simulated => self.state.provider.available_balance(),
            ProviderMode::ManualPrepaidCard => self
                .state
                .manual_provider
                .as_ref()
                .and_then(|provider| provider.balance_snapshot.as_ref())
                .map(|snapshot| snapshot.amount.clone())
                .ok_or_else(|| {
                    TreasuryError::Conflict("manual balance snapshot is unavailable".to_string())
                }),
        }
    }

    fn provider_balance_status(&self) -> &'static str {
        match self.state.provider_mode {
            ProviderMode::Simulated => "simulated_provider_verified",
            ProviderMode::ManualPrepaidCard => match self
                .state
                .manual_provider
                .as_ref()
                .and_then(|provider| provider.balance_snapshot.as_ref())
                .map(|snapshot| &snapshot.status)
            {
                Some(BalanceStatus::Estimated) => "estimated",
                Some(BalanceStatus::OwnerConfirmed) => "owner_confirmed",
                Some(BalanceStatus::ProviderVerified) => "provider_verified",
                None => "unavailable",
            },
        }
    }

    fn provider_authorize(&mut self, intent: &PurchaseIntent) -> Result<ProviderOutcome> {
        match self.state.provider_mode {
            ProviderMode::Simulated => self.state.provider.authorize(intent),
            ProviderMode::ManualPrepaidCard => self
                .state
                .manual_provider
                .as_mut()
                .ok_or_else(|| {
                    TreasuryError::Conflict("manual provider is not configured".to_string())
                })?
                .authorize(intent),
        }
    }

    pub fn handle_rpc(&mut self, request: RpcRequest) -> RpcResponse {
        let request_id = request.request_id.clone();
        if request.api_version != API_VERSION {
            return RpcResponse {
                api_version: API_VERSION.to_string(),
                request_id,
                ok: false,
                data: None,
                error: Some("unsupported api version".to_string()),
            };
        }
        match self.handle(&request.token, request.operation) {
            Ok(data) => RpcResponse {
                api_version: API_VERSION.to_string(),
                request_id,
                ok: true,
                data: Some(data),
                error: None,
            },
            Err(error) => RpcResponse {
                api_version: API_VERSION.to_string(),
                request_id,
                ok: false,
                data: None,
                error: Some(redact_sensitive(&error.to_string())),
            },
        }
    }

    pub fn handle_rpc_persisted(&mut self, request: RpcRequest, data_dir: &Path) -> RpcResponse {
        let request_id = request.request_id.clone();
        if request.api_version != API_VERSION {
            return RpcResponse {
                api_version: API_VERSION.to_string(),
                request_id,
                ok: false,
                data: None,
                error: Some("unsupported api version".to_string()),
            };
        }
        match self.handle_persisted(&request.token, request.operation, data_dir) {
            Ok(data) => RpcResponse {
                api_version: API_VERSION.to_string(),
                request_id,
                ok: true,
                data: Some(data),
                error: None,
            },
            Err(error) => RpcResponse {
                api_version: API_VERSION.to_string(),
                request_id,
                ok: false,
                data: None,
                error: Some(redact_sensitive(&error.to_string())),
            },
        }
    }

    pub fn handle_persisted(
        &mut self,
        token: &str,
        request: Request,
        data_dir: &Path,
    ) -> Result<Value> {
        match request {
            Request::ExecutePurchaseIntent { intent_id } => {
                let actor = self.authenticate(token)?;
                let snapshot = self.state.clone();
                if let Err(error) = self.prepare_intent_execution(&actor, &intent_id) {
                    self.state = snapshot;
                    return Err(error);
                }
                if let Err(error) = self.save_to(data_dir) {
                    self.state = snapshot;
                    return Err(error);
                }
                match self.complete_intent_execution(&actor, &intent_id) {
                    Ok(value) => {
                        self.save_to(data_dir)?;
                        Ok(value)
                    }
                    Err(error) => {
                        self.quarantine_execution(&intent_id, &error.to_string())?;
                        self.save_to(data_dir)?;
                        Err(error)
                    }
                }
            }
            request => {
                let snapshot = self.state.clone();
                match self.handle(token, request) {
                    Ok(value) => {
                        if let Err(error) = self.save_to(data_dir) {
                            self.state = snapshot;
                            return Err(error);
                        }
                        Ok(value)
                    }
                    Err(error) => {
                        self.state = snapshot;
                        Err(error)
                    }
                }
            }
        }
    }

    pub fn handle(&mut self, token: &str, request: Request) -> Result<Value> {
        let actor = self.authenticate(token)?;
        match request {
            Request::GetStatus => self.get_status(&actor),
            Request::GetCapabilities => self.get_capabilities(&actor),
            Request::GetBudget => self.get_budget(&actor),
            Request::GetReceiveInstructions => self.get_receive_instructions(&actor),
            Request::CreatePurchaseIntent { request } => self.create_intent(&actor, request),
            Request::GetPurchaseIntent { intent_id } => self.get_intent(&actor, &intent_id),
            Request::ExecutePurchaseIntent { intent_id } => self.execute_intent(&actor, &intent_id),
            Request::CancelPurchaseIntent { intent_id } => self.cancel_intent(&actor, &intent_id),
            Request::ListTransactions => self.list_transactions(&actor),
            Request::GetReceipt { intent_id } => self.get_receipt(&actor, &intent_id),
            Request::OwnerCreateAgent { name, policy, mode, ttl_secs } => {
                self.owner_create_agent(&actor, name, policy, mode, ttl_secs)
            }
            Request::OwnerUpdatePolicy { agent_id, policy } => {
                self.owner_update_policy(&actor, &agent_id, policy)
            }
            Request::OwnerSetAgentMode { agent_id, mode } => {
                self.owner_set_agent_mode(&actor, &agent_id, mode)
            }
            Request::OwnerRevokeAgent { agent_id } => self.owner_revoke_agent(&actor, &agent_id),
            Request::OwnerSetEmergencyStop { stopped } => {
                self.owner_emergency_stop(&actor, stopped)
            }
            Request::OwnerApproveIntent { intent_id } => self.owner_approve(&actor, &intent_id),
            Request::OwnerReconcile { intent_id, outcome, provider_reference } => {
                self.owner_reconcile(&actor, &intent_id, outcome, provider_reference)
            }
            Request::OwnerRecordDeposit {
                amount,
                source,
                verified,
                agent_id,
                external_reference,
            } => self.owner_record_deposit(
                &actor,
                amount,
                source,
                verified,
                agent_id,
                external_reference,
            ),
            Request::OwnerArmAgentSession { agent_id, ttl_secs } => {
                self.owner_arm_agent_session(&actor, &agent_id, ttl_secs)
            }
            Request::OwnerConfigureManualProvider {
                credential_reference,
                provider_kind,
                last_four,
                balance,
                balance_status,
                balance_ttl_secs,
            } => self.owner_configure_manual_provider(
                &actor,
                ManualProviderConfiguration {
                    credential_reference,
                    provider_kind,
                    last_four,
                    balance,
                    balance_status,
                    balance_ttl_secs,
                },
            ),
            Request::OwnerConfigureReceiveInstructions { method, address, memo_template } => {
                self.owner_configure_receive(&actor, method, address, memo_template)
            }
            Request::OwnerListAudit => self.owner_list_audit(&actor),
        }
    }

    fn authenticate(&self, token: &str) -> Result<Actor> {
        if token.is_empty() || token.len() > 128 {
            return Err(TreasuryError::Unauthorized);
        }
        let hash = token_hash(token);
        if hash == self.state.owner.capability_token_hash {
            return Ok(Actor::Owner);
        }
        let at = now();
        self.state
            .agents
            .values()
            .find(|agent| {
                agent.capability_token_hash == hash && !agent.revoked && agent.expires_at > at
            })
            .map(|agent| Actor::Agent(agent.id.clone()))
            .ok_or(TreasuryError::Unauthorized)
    }

    fn require_owner(actor: &Actor) -> Result<()> {
        if *actor != Actor::Owner {
            return Err(TreasuryError::Forbidden(
                "this operation is owner-only and is never exposed to agents".to_string(),
            ));
        }
        Ok(())
    }

    fn require_agent(actor: &Actor) -> Result<&str> {
        match actor {
            Actor::Agent(id) => Ok(id),
            Actor::Owner => Err(TreasuryError::Forbidden(
                "this operation requires an agent capability".to_string(),
            )),
        }
    }

    fn require_agent_scope<'a>(&'a self, actor: &'a Actor, scope: &str) -> Result<&'a str> {
        let agent_id = Self::require_agent(actor)?;
        if !self.agent(agent_id)?.scopes.contains(scope) {
            return Err(TreasuryError::Forbidden(format!("agent scope {scope} is missing")));
        }
        Ok(agent_id)
    }

    fn get_status(&self, actor: &Actor) -> Result<Value> {
        match actor {
            Actor::Owner => Ok(json!({
                "principal": "owner",
                "emergency_stop": self.state.emergency_stop,
                "agent_count": self.state.agents.len(),
                "audit_entries": self.state.audit.len(),
                "provider": self.provider_id(),
            })),
            Actor::Agent(agent_id) => {
                self.require_agent_scope(actor, "status:read")?;
                let agent = self.agent(agent_id)?;
                Ok(json!({
                    "principal": "agent",
                    "agent_id": agent.id,
                    "mode": agent.mode,
                    "revoked": agent.revoked,
                    "emergency_stop": self.state.emergency_stop,
                    "policy_id": agent.policy_id,
                    "broker_session_expires_at": agent.broker_session_expires_at,
                }))
            }
        }
    }

    fn get_capabilities(&self, actor: &Actor) -> Result<Value> {
        let agent_id = self.require_agent_scope(actor, "capabilities:read")?;
        let agent = self.agent(agent_id)?;
        Ok(json!({
            "agent_id": agent.id,
            "scopes": agent.scopes,
            "cannot": [
                "view_credentials",
                "change_policies",
                "change_limits",
                "add_cards",
                "approve_exceptions",
                "record_deposits",
                "reconcile_transactions",
                "export_sensitive_data",
                "change_server_configuration",
                "disable_safeguards",
            ],
        }))
    }

    fn get_budget(&self, actor: &Actor) -> Result<Value> {
        let agent_id = self.require_agent_scope(actor, "budget:read")?;
        let agent = self.agent(agent_id)?;
        let policy = self.policy(&agent.policy_id)?;
        let usage = self.usage(agent_id, &policy.primary_currency)?;
        let provider_balance = self.provider_reported_balance()?;
        let ledger = self.ledger_snapshot(&policy.primary_currency)?;
        let remaining_transaction = policy.max_per_transaction.clone();
        let remaining_session = remaining_money(&policy.max_per_session, &usage.session_amount)?;
        let remaining_rolling =
            remaining_money(&policy.max_rolling_24h, &usage.rolling_24h_amount)?;
        let remaining_lifetime = remaining_money(&policy.max_lifetime, &usage.lifetime_amount)?;
        Ok(json!({
            "currency": policy.primary_currency,
            "policy_version": policy.version,
            "mode": agent.mode,
            "policy_budget": {
                "max_per_transaction": policy.max_per_transaction,
                "max_per_session": policy.max_per_session,
                "max_rolling_24h": policy.max_rolling_24h,
                "max_lifetime": policy.max_lifetime,
            },
            "used": usage,
            "remaining": {
                "per_transaction": remaining_transaction,
                "session": remaining_session,
                "rolling_24h": remaining_rolling,
                "lifetime": remaining_lifetime,
            },
            "provider_balance": {
                "amount": provider_balance,
                "status": self.provider_balance_status(),
            },
            "ledger": ledger,
            "estimated_balance_is_not_provider_verified": true,
        }))
    }

    fn get_receive_instructions(&self, actor: &Actor) -> Result<Value> {
        self.require_agent_scope(actor, "receive:read")?;
        let instructions =
            self.state.receive_instructions.as_ref().filter(|value| value.public).ok_or_else(
                || TreasuryError::NotFound("public receiving instructions".to_string()),
            )?;
        Ok(json!({
            "method": instructions.method,
            "address": instructions.address,
            "memo_template": instructions.memo_template,
            "verified": false,
            "warning": "A notification or screenshot is not evidence of received funds.",
            "outgoing_transfers_supported": false,
        }))
    }

    fn create_intent(&mut self, actor: &Actor, request: PurchaseRequest) -> Result<Value> {
        let agent_id = self.require_agent_scope(actor, "purchase_intents:create")?.to_string();
        let agent = self.agent(&agent_id)?.clone();
        if let Some(existing) = self.state.intents.values().find(|intent| {
            intent.agent_id == agent_id && intent.request.idempotency_key == request.idempotency_key
        }) {
            if existing.request == request {
                return Ok(self.sanitized_intent(existing));
            }
            return Err(TreasuryError::Conflict(
                "idempotency key is already bound to another request".to_string(),
            ));
        }
        let policy = self.policy(&agent.policy_id)?.clone();
        if agent.broker_session_expires_at <= now() {
            return Err(TreasuryError::Forbidden(
                "agent spending session has expired and must be re-armed by the owner".to_string(),
            ));
        }
        let usage = self.usage(&agent_id, &policy.primary_currency)?;
        let merchant = canonicalize_domain(&request.merchant_domain)?;
        let known_merchant = policy.allowed_merchants.contains(&merchant)
            || agent.approved_merchants.contains(&merchant);
        let mut decision = policy.evaluate(
            &request,
            &usage,
            &agent.mode,
            known_merchant,
            self.state.emergency_stop,
            now(),
        )?;
        self.enforce_authority(&mut decision, &request, &usage, &policy)?;
        if self.state.provider_mode == ProviderMode::ManualPrepaidCard {
            decision.allowed = false;
            decision.requires_approval = true;
            decision.reasons = vec![
                "manual prepaid-card checkout requires an authenticated owner handoff".to_string(),
            ];
        }
        let id = new_id("intent");
        let at = now();
        let mut intent = PurchaseIntent {
            id: id.clone(),
            agent_id: agent_id.clone(),
            broker_session_id: agent.broker_session_id.clone(),
            request,
            state: TransactionState::Draft,
            decision: decision.clone(),
            policy_version: policy.version,
            created_at: at,
            updated_at: at,
            provider_reference: None,
            receipt_hash: None,
            last_error: None,
        };
        intent.transition(TransactionState::Proposed)?;
        if !decision.reasons.is_empty() && !decision.requires_approval {
            intent.last_error = Some(decision.reasons.join("; "));
            intent.transition(TransactionState::Failed)?;
        } else if decision.requires_approval {
            intent.transition(TransactionState::ApprovalRequired)?;
        } else {
            intent.transition(TransactionState::PolicyValidated)?;
        }
        self.audit(
            &agent_id,
            "create_purchase_intent",
            Some(&id),
            Some(policy.version),
            Some(if intent.state == TransactionState::PolicyValidated {
                "allowed"
            } else if intent.state == TransactionState::ApprovalRequired {
                "approval_required"
            } else {
                "denied"
            }),
            json!({
                "merchant_domain": intent.request.merchant_domain,
                "amount": intent.request.amount,
                "reasons": decision.reasons,
            }),
        )?;
        self.state.intents.insert(id, intent.clone());
        Ok(self.sanitized_intent(&intent))
    }

    fn execute_intent(&mut self, actor: &Actor, intent_id: &str) -> Result<Value> {
        let snapshot = self.state.clone();
        if let Err(error) = self.prepare_intent_execution(actor, intent_id) {
            self.state = snapshot;
            return Err(error);
        }
        match self.complete_intent_execution(actor, intent_id) {
            Ok(value) => Ok(value),
            Err(error) => {
                self.quarantine_execution(intent_id, &error.to_string())?;
                Err(error)
            }
        }
    }

    fn prepare_intent_execution(&mut self, actor: &Actor, intent_id: &str) -> Result<()> {
        let agent_id = self.require_agent_scope(actor, "purchase_intents:execute")?.to_string();
        let existing = self
            .state
            .intents
            .get(intent_id)
            .ok_or_else(|| TreasuryError::NotFound(format!("purchase intent {intent_id}")))?;
        if existing.agent_id != agent_id {
            return Err(TreasuryError::Forbidden("intent belongs to another agent".to_string()));
        }
        if existing.state == TransactionState::Unknown
            || existing.state == TransactionState::ProviderPending
        {
            return Err(TreasuryError::Conflict(
                "ambiguous or pending payment requires owner reconciliation; retry is disabled"
                    .to_string(),
            ));
        }
        if existing.state == TransactionState::ApprovalRequired {
            return Err(TreasuryError::Forbidden(
                "owner approval is required before execution".to_string(),
            ));
        }
        if existing.state != TransactionState::PolicyValidated
            && existing.state != TransactionState::Approved
        {
            return Err(TreasuryError::Conflict(format!(
                "intent is not executable in state {:?}",
                existing.state
            )));
        }
        let agent = self.agent(&agent_id)?.clone();
        let policy = self.policy(&agent.policy_id)?.clone();
        if agent.broker_session_expires_at <= now()
            || existing.broker_session_id != agent.broker_session_id
        {
            return Err(TreasuryError::Forbidden(
                "owner-armed spending session is expired or no longer current".to_string(),
            ));
        }
        if now() > existing.created_at + policy.intent_ttl_secs
            || now() > existing.created_at + policy.card_session_ttl_secs
        {
            return Err(TreasuryError::Conflict(
                "purchase intent or card session has expired; create a new intent".to_string(),
            ));
        }
        let usage = self.usage(&agent_id, &policy.primary_currency)?;
        let merchant = canonicalize_domain(&existing.request.merchant_domain)?;
        let known_merchant = policy.allowed_merchants.contains(&merchant)
            || agent.approved_merchants.contains(&merchant);
        let mut decision = policy.evaluate(
            &existing.request,
            &usage,
            &agent.mode,
            known_merchant,
            self.state.emergency_stop,
            now(),
        )?;
        self.enforce_authority(&mut decision, &existing.request, &usage, &policy)?;
        if !(decision.allowed
            || existing.state == TransactionState::Approved && decision.requires_approval)
        {
            return Err(TreasuryError::Forbidden(format!(
                "pre-submit policy validation failed: {}",
                decision.reasons.join("; ")
            )));
        }
        let mut intent = existing.clone();
        intent.transition(TransactionState::FundsReserved)?;
        self.append_ledger(
            LedgerEventKind::HoldReserved,
            intent.request.amount.clone(),
            Some(intent.id.clone()),
            "policy-reservation",
            true,
        )?;
        self.audit(
            &agent_id,
            "reserve_funds",
            Some(intent_id),
            Some(policy.version),
            Some("allowed"),
            json!({ "amount": intent.request.amount }),
        )?;
        intent.transition(TransactionState::Executing)?;
        self.state.intents.insert(intent.id.clone(), intent);
        Ok(())
    }

    fn complete_intent_execution(&mut self, actor: &Actor, intent_id: &str) -> Result<Value> {
        let agent_id = self.require_agent_scope(actor, "purchase_intents:execute")?.to_string();
        let mut intent = self.intent_for_agent(&agent_id, intent_id)?.clone();
        if intent.state != TransactionState::Executing {
            return Err(TreasuryError::Conflict(
                "provider completion requires an executing intent".to_string(),
            ));
        }
        let policy_version = intent.policy_version;
        let outcome = self.provider_authorize(&intent)?;
        let response = match outcome {
            ProviderOutcome::Approved { reference } => {
                intent.provider_reference = Some(sanitize_provider_reference(&reference));
                intent.transition(TransactionState::Settled)?;
                let receipt = self.receipt(&intent)?;
                intent.receipt_hash = Some(Self::receipt_hash(&receipt)?);
                self.record_settled(&intent)?;
                json!({ "status": "settled", "intent": self.sanitized_intent(&intent), "receipt": receipt })
            }
            ProviderOutcome::Declined { reason: _ } => {
                intent.last_error = Some("provider_declined".to_string());
                intent.transition(TransactionState::Declined)?;
                self.append_ledger(
                    LedgerEventKind::HoldReleased,
                    intent.request.amount.clone(),
                    Some(intent.id.clone()),
                    "provider-decline",
                    true,
                )?;
                json!({ "status": "declined", "intent": self.sanitized_intent(&intent) })
            }
            ProviderOutcome::Pending { reference } => {
                intent.provider_reference = Some(sanitize_provider_reference(&reference));
                intent.transition(TransactionState::ProviderPending)?;
                json!({ "status": "provider_pending", "intent": self.sanitized_intent(&intent), "action": "owner_reconcile" })
            }
            ProviderOutcome::Unknown { reason: _ } => {
                intent.last_error = Some("provider_outcome_unknown".to_string());
                intent.transition(TransactionState::Unknown)?;
                json!({ "status": "unknown", "intent": self.sanitized_intent(&intent), "action": "owner_reconcile", "retry": false })
            }
        };
        self.audit(
            &agent_id,
            "provider_outcome",
            Some(intent_id),
            Some(policy_version),
            Some(match &intent.state {
                TransactionState::Settled => "settled",
                TransactionState::Declined => "declined",
                TransactionState::ProviderPending => "provider_pending",
                TransactionState::Unknown => "unknown",
                _ => "unexpected",
            }),
            json!({ "state": intent.state, "provider_reference": intent.provider_reference }),
        )?;
        self.state.intents.insert(intent.id.clone(), intent.clone());
        Ok(response)
    }

    fn quarantine_execution(&mut self, intent_id: &str, reason: &str) -> Result<()> {
        let mut intent = self
            .state
            .intents
            .get(intent_id)
            .ok_or_else(|| TreasuryError::NotFound(intent_id.to_string()))?
            .clone();
        if intent.state == TransactionState::Executing {
            intent.transition(TransactionState::Unknown)?;
        } else if intent.state != TransactionState::Unknown {
            return Ok(());
        }
        intent.last_error = Some("provider_execution_error".to_string());
        self.state.intents.insert(intent.id.clone(), intent.clone());
        self.audit(
            "broker",
            "quarantine_provider_error",
            Some(intent_id),
            Some(intent.policy_version),
            Some("unknown"),
            json!({ "reason": redact_sensitive(reason), "retry": false }),
        )
    }

    fn cancel_intent(&mut self, actor: &Actor, intent_id: &str) -> Result<Value> {
        let agent_id = self.require_agent_scope(actor, "purchase_intents:cancel")?.to_string();
        let mut intent = self.intent_for_agent(&agent_id, intent_id)?.clone();
        if !matches!(
            intent.state,
            TransactionState::Proposed
                | TransactionState::ApprovalRequired
                | TransactionState::PolicyValidated
        ) {
            return Err(TreasuryError::Conflict(
                "only unexecuted intents can be cancelled".to_string(),
            ));
        }
        intent.transition(TransactionState::Cancelled)?;
        self.state.intents.insert(intent.id.clone(), intent.clone());
        self.audit(
            &agent_id,
            "cancel_purchase_intent",
            Some(intent_id),
            None,
            Some("allowed"),
            json!({}),
        )?;
        Ok(self.sanitized_intent(&intent))
    }

    fn get_intent(&self, actor: &Actor, intent_id: &str) -> Result<Value> {
        if matches!(actor, Actor::Agent(_)) {
            self.require_agent_scope(actor, "purchase_intents:read")?;
        }
        let intent = self
            .state
            .intents
            .get(intent_id)
            .ok_or_else(|| TreasuryError::NotFound(intent_id.to_string()))?;
        if let Actor::Agent(agent_id) = actor
            && intent.agent_id != *agent_id
        {
            return Err(TreasuryError::Forbidden("intent belongs to another agent".to_string()));
        }
        Ok(self.sanitized_intent(intent))
    }

    fn list_transactions(&self, actor: &Actor) -> Result<Value> {
        if matches!(actor, Actor::Agent(_)) {
            self.require_agent_scope(actor, "transactions:read")?;
        }
        let values: Vec<Value> = self
            .state
            .intents
            .values()
            .filter(|intent| match actor {
                Actor::Owner => true,
                Actor::Agent(agent_id) => intent.agent_id == *agent_id,
            })
            .map(|intent| self.sanitized_intent(intent))
            .collect();
        Ok(json!({ "transactions": values }))
    }

    fn get_receipt(&self, actor: &Actor, intent_id: &str) -> Result<Value> {
        if matches!(actor, Actor::Agent(_)) {
            self.require_agent_scope(actor, "receipts:read")?;
        }
        let intent = self
            .state
            .intents
            .get(intent_id)
            .ok_or_else(|| TreasuryError::NotFound(intent_id.to_string()))?;
        if let Actor::Agent(agent_id) = actor
            && intent.agent_id != *agent_id
        {
            return Err(TreasuryError::Forbidden("intent belongs to another agent".to_string()));
        }
        self.receipt(intent).map(|value| json!(value))
    }

    fn owner_create_agent(
        &mut self,
        actor: &Actor,
        name: String,
        mut policy: Policy,
        mode: AutonomyMode,
        ttl_secs: i64,
    ) -> Result<Value> {
        Self::require_owner(actor)?;
        bounded(&name, "agent_name", 128)?;
        policy.validate()?;
        if ttl_secs <= 0 || ttl_secs > 24 * 60 * 60 {
            return Err(TreasuryError::Invalid(
                "agent TTL must be between 1 second and 24 hours".to_string(),
            ));
        }
        let token = new_token();
        let agent_id = new_id("agent");
        policy.id = format!("policy_{agent_id}");
        policy.version = 1;
        let policy_id = policy.id.clone();
        let session_ttl = ttl_secs.min(policy.card_session_ttl_secs);
        let scopes = [
            "status:read",
            "capabilities:read",
            "budget:read",
            "receive:read",
            "purchase_intents:create",
            "purchase_intents:read",
            "purchase_intents:execute",
            "purchase_intents:cancel",
            "transactions:read",
            "receipts:read",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        let agent = AgentRecord {
            id: agent_id.clone(),
            name,
            capability_token_hash: token_hash(&token),
            scopes,
            policy_id: policy_id.clone(),
            approved_merchants: BTreeSet::new(),
            broker_session_id: new_id("session"),
            broker_session_expires_at: now() + session_ttl,
            mode,
            created_at: now(),
            expires_at: now() + ttl_secs,
            revoked: false,
        };
        self.state.policies.insert(policy_id, policy);
        self.state.agents.insert(agent_id.clone(), agent.clone());
        self.audit(
            "owner",
            "create_agent",
            None,
            Some(1),
            Some("allowed"),
            json!({ "agent_id": agent_id, "broker_session_expires_at": agent.broker_session_expires_at }),
        )?;
        Ok(
            json!({ "agent_id": agent.id, "capability_token": token, "expires_at": agent.expires_at, "broker_session_expires_at": agent.broker_session_expires_at, "warning": "Store this token once in a protected file. It is never shown again and is not an owner credential." }),
        )
    }

    fn owner_arm_agent_session(
        &mut self,
        actor: &Actor,
        agent_id: &str,
        ttl_secs: i64,
    ) -> Result<Value> {
        Self::require_owner(actor)?;
        let agent = self.agent(agent_id)?.clone();
        let policy = self.policy(&agent.policy_id)?;
        let policy_version = policy.version;
        let maximum_session_ttl = policy.card_session_ttl_secs;
        if ttl_secs <= 0 || ttl_secs > maximum_session_ttl {
            return Err(TreasuryError::Invalid(
                "session TTL must be positive and no longer than the policy card-session TTL"
                    .to_string(),
            ));
        }
        let expires_at = (now() + ttl_secs).min(agent.expires_at);
        let session_id = new_id("session");
        let target = self
            .state
            .agents
            .get_mut(agent_id)
            .ok_or_else(|| TreasuryError::NotFound(format!("agent {agent_id}")))?;
        target.broker_session_id = session_id.clone();
        target.broker_session_expires_at = expires_at;
        self.audit(
            "owner",
            "arm_agent_session",
            None,
            Some(policy_version),
            Some("allowed"),
            json!({ "agent_id": agent_id, "session_id": session_id, "expires_at": expires_at }),
        )?;
        Ok(json!({ "agent_id": agent_id, "session_id": session_id, "expires_at": expires_at }))
    }

    fn owner_update_policy(
        &mut self,
        actor: &Actor,
        agent_id: &str,
        mut policy: Policy,
    ) -> Result<Value> {
        Self::require_owner(actor)?;
        policy.validate()?;
        let agent = self.agent(agent_id)?.clone();
        let current = self.policy(&agent.policy_id)?.clone();
        policy.id = agent.policy_id.clone();
        policy.version = current.version + 1;
        self.state.policies.insert(policy.clone().id.clone(), policy.clone());
        self.audit(
            "owner",
            "update_policy",
            None,
            Some(policy.version),
            Some("allowed"),
            json!({ "agent_id": agent_id, "policy_version": policy.version }),
        )?;
        Ok(
            json!({ "agent_id": agent_id, "policy": policy, "note": "Existing rejected intents remain rejected until a new intent is created." }),
        )
    }

    fn owner_emergency_stop(&mut self, actor: &Actor, stopped: bool) -> Result<Value> {
        Self::require_owner(actor)?;
        self.state.emergency_stop = stopped;
        self.audit(
            "owner",
            if stopped { "emergency_stop_on" } else { "emergency_stop_off" },
            None,
            None,
            Some("allowed"),
            json!({ "stopped": stopped }),
        )?;
        Ok(json!({ "emergency_stop": stopped, "pending_operations_resume_automatically": false }))
    }

    fn owner_set_agent_mode(
        &mut self,
        actor: &Actor,
        agent_id: &str,
        mode: AutonomyMode,
    ) -> Result<Value> {
        Self::require_owner(actor)?;
        let agent = self
            .state
            .agents
            .get_mut(agent_id)
            .ok_or_else(|| TreasuryError::NotFound(format!("agent {agent_id}")))?;
        agent.mode = mode.clone();
        self.audit(
            "owner",
            "set_agent_mode",
            None,
            None,
            Some("allowed"),
            json!({ "agent_id": agent_id, "mode": mode }),
        )?;
        Ok(json!({ "agent_id": agent_id, "mode": mode }))
    }

    fn owner_revoke_agent(&mut self, actor: &Actor, agent_id: &str) -> Result<Value> {
        Self::require_owner(actor)?;
        let agent = self
            .state
            .agents
            .get_mut(agent_id)
            .ok_or_else(|| TreasuryError::NotFound(format!("agent {agent_id}")))?;
        agent.revoked = true;
        agent.mode = AutonomyMode::Disabled;
        self.audit(
            "owner",
            "revoke_agent",
            None,
            None,
            Some("allowed"),
            json!({ "agent_id": agent_id }),
        )?;
        Ok(json!({ "agent_id": agent_id, "revoked": true, "mode": "disabled" }))
    }

    fn owner_approve(&mut self, actor: &Actor, intent_id: &str) -> Result<Value> {
        Self::require_owner(actor)?;
        let mut intent = self
            .state
            .intents
            .get(intent_id)
            .ok_or_else(|| TreasuryError::NotFound(intent_id.to_string()))?
            .clone();
        if intent.state != TransactionState::ApprovalRequired {
            return Err(TreasuryError::Conflict("intent is not awaiting approval".to_string()));
        }
        let merchant = canonicalize_domain(&intent.request.merchant_domain)?;
        self.state
            .agents
            .get_mut(&intent.agent_id)
            .ok_or_else(|| TreasuryError::NotFound(format!("agent {}", intent.agent_id)))?
            .approved_merchants
            .insert(merchant.clone());
        intent.transition(TransactionState::Approved)?;
        self.state.intents.insert(intent.id.clone(), intent.clone());
        self.audit(
            "owner",
            "approve_intent",
            Some(intent_id),
            Some(intent.policy_version),
            Some("allowed"),
            json!({ "owner_action": true, "approved_merchant": merchant }),
        )?;
        Ok(self.sanitized_intent(&intent))
    }

    fn owner_reconcile(
        &mut self,
        actor: &Actor,
        intent_id: &str,
        outcome: ReconciliationOutcome,
        provider_reference: Option<String>,
    ) -> Result<Value> {
        Self::require_owner(actor)?;
        let snapshot = self.state.clone();
        match self.owner_reconcile_inner(intent_id, outcome, provider_reference) {
            Ok(value) => Ok(value),
            Err(error) => {
                self.state = snapshot;
                Err(error)
            }
        }
    }

    fn owner_reconcile_inner(
        &mut self,
        intent_id: &str,
        outcome: ReconciliationOutcome,
        provider_reference: Option<String>,
    ) -> Result<Value> {
        let provider_reference =
            provider_reference.as_deref().map(validate_provider_reference).transpose()?;
        let mut intent = self
            .state
            .intents
            .get(intent_id)
            .ok_or_else(|| TreasuryError::NotFound(intent_id.to_string()))?
            .clone();
        let was_settled = intent.state == TransactionState::Settled;
        let uncertain = matches!(
            intent.state,
            TransactionState::Unknown
                | TransactionState::ProviderPending
                | TransactionState::ReconciliationRequired
        );
        if !(uncertain
            || outcome == ReconciliationOutcome::Refunded
                && intent.state == TransactionState::Settled)
        {
            return Err(TreasuryError::Conflict(
                "intent is not eligible for the requested reconciliation".to_string(),
            ));
        }
        if uncertain
            && (intent.state == TransactionState::Unknown
                || intent.state == TransactionState::ProviderPending)
        {
            intent.transition(TransactionState::ReconciliationRequired)?;
        }
        match outcome {
            ReconciliationOutcome::Settled => {
                let reference = match self.state.provider_mode {
                    ProviderMode::Simulated => {
                        let reference = provider_reference
                            .clone()
                            .or_else(|| intent.provider_reference.clone())
                            .unwrap_or_else(|| new_id("reconciled"));
                        self.state.provider.reconcile_settled(&intent, &reference)?
                    }
                    ProviderMode::ManualPrepaidCard => {
                        let reference = provider_reference.clone().ok_or_else(|| {
                            TreasuryError::Invalid(
                                "manual settlement requires an owner-verified provider reference"
                                    .to_string(),
                            )
                        })?;
                        let provider = self.state.manual_provider.as_mut().ok_or_else(|| {
                            TreasuryError::Conflict("manual provider is not configured".to_string())
                        })?;
                        let snapshot = provider.balance_snapshot.as_mut().ok_or_else(|| {
                            TreasuryError::Conflict(
                                "manual balance snapshot is unavailable".to_string(),
                            )
                        })?;
                        snapshot.amount = snapshot.amount.checked_sub(&intent.request.amount)?;
                        snapshot.observed_at = now();
                        reference
                    }
                };
                intent.provider_reference = Some(reference);
                intent.transition(TransactionState::Settled)?;
                self.record_settled(&intent)?;
            }
            ReconciliationOutcome::Declined => {
                if self.state.provider_mode == ProviderMode::Simulated
                    && self.state.provider.charges.contains_key(&intent.id)
                {
                    return Err(TreasuryError::Conflict(
                        "a known provider charge cannot be reconciled as declined".to_string(),
                    ));
                }
                if self.state.provider_mode == ProviderMode::Simulated {
                    self.state.provider.release_hold(&intent.id);
                }
                intent.transition(TransactionState::Declined)?;
                self.append_ledger(
                    LedgerEventKind::HoldReleased,
                    intent.request.amount.clone(),
                    Some(intent.id.clone()),
                    "owner-reconciliation",
                    true,
                )?;
            }
            ReconciliationOutcome::Refunded => {
                if intent.state != TransactionState::Settled {
                    let reference = provider_reference.clone().ok_or_else(|| {
                        TreasuryError::Invalid(
                            "refund reconciliation requires a provider reference".to_string(),
                        )
                    })?;
                    intent.provider_reference = Some(reference);
                    intent.transition(TransactionState::Settled)?;
                    self.record_settled(&intent)?;
                }
                let refund = match self.state.provider_mode {
                    ProviderMode::Simulated => self.state.provider.refund(&intent.id)?,
                    ProviderMode::ManualPrepaidCard => {
                        let provider = self.state.manual_provider.as_mut().ok_or_else(|| {
                            TreasuryError::Conflict("manual provider is not configured".to_string())
                        })?;
                        let snapshot = provider.balance_snapshot.as_mut().ok_or_else(|| {
                            TreasuryError::Conflict(
                                "manual balance snapshot is unavailable".to_string(),
                            )
                        })?;
                        if was_settled {
                            snapshot.amount =
                                snapshot.amount.checked_add(&intent.request.amount)?;
                        }
                        snapshot.observed_at = now();
                        intent.request.amount.clone()
                    }
                };
                intent.transition(TransactionState::Refunded)?;
                self.append_ledger(
                    LedgerEventKind::Refund,
                    refund,
                    Some(intent.id.clone()),
                    "owner_reconciliation",
                    true,
                )?;
            }
        }
        self.state.intents.insert(intent.id.clone(), intent.clone());
        self.audit(
            "owner",
            "reconcile_intent",
            Some(intent_id),
            Some(intent.policy_version),
            Some("allowed"),
            json!({ "outcome": outcome, "provider_reference": intent.provider_reference }),
        )?;
        Ok(self.sanitized_intent(&intent))
    }

    fn owner_record_deposit(
        &mut self,
        actor: &Actor,
        amount: Money,
        source: String,
        verified: bool,
        agent_id: Option<String>,
        external_reference: String,
    ) -> Result<Value> {
        Self::require_owner(actor)?;
        bounded(&source, "deposit_source", 128)?;
        bounded(&external_reference, "external_reference", 160)?;
        let external_reference = opaque_reference("deposit", &external_reference);
        let source = redact_sensitive(&source);
        if amount.minor <= 0 {
            return Err(TreasuryError::Invalid("deposit must be positive".to_string()));
        }
        let fingerprint = {
            let value = json!({
                "amount": amount,
                "source": source,
                "verified": verified,
                "agent_id": agent_id,
            });
            let mut digest = Sha256::new();
            digest.update(serde_json::to_vec(&value)?);
            hex::encode(digest.finalize())
        };
        if let Some(existing) = self.state.processed_deposits.get(&external_reference) {
            if existing == &fingerprint {
                return Ok(json!({
                    "recorded": true,
                    "duplicate": true,
                    "external_reference": external_reference,
                }));
            }
            return Err(TreasuryError::Conflict(
                "external deposit reference is already bound to different details".to_string(),
            ));
        }
        let policy = if verified {
            let agent_id = agent_id.as_ref().ok_or_else(|| {
                TreasuryError::Invalid(
                    "verified deposits require an explicit target agent policy".to_string(),
                )
            })?;
            Some(self.policy(&self.agent(agent_id)?.policy_id)?.clone())
        } else {
            None
        };
        let reinvested = if verified {
            let policy = policy.as_ref().ok_or_else(|| {
                TreasuryError::Conflict("verified income requires an applicable policy".to_string())
            })?;
            if amount.currency != policy.primary_currency {
                return Err(TreasuryError::Money(
                    "deposit currency does not match policy currency".to_string(),
                ));
            }
            let current_provider_balance = self.provider_reported_balance()?;
            let projected_provider_balance = current_provider_balance.checked_add(&amount)?;
            if projected_provider_balance.minor > policy.max_treasury_size.minor {
                return Err(TreasuryError::Forbidden(
                    "maximum treasury size would be exceeded".to_string(),
                ));
            }
            let reinvested = amount.scaled_basis_points(policy.reinvestment_ratio_bps)?;
            let current_authority =
                self.ledger_snapshot(&policy.primary_currency)?.available_authority;
            if current_authority.checked_add(&reinvested)?.minor
                > policy.absolute_exposure_ceiling.minor
            {
                return Err(TreasuryError::Forbidden(
                    "absolute exposure ceiling would be exceeded".to_string(),
                ));
            }
            reinvested
        } else {
            Money::zero(&amount.currency)?
        };
        if verified {
            match self.state.provider_mode {
                ProviderMode::Simulated => {
                    self.state.provider.balance =
                        self.state.provider.balance.checked_add(&amount)?;
                    self.state.provider.incoming_deposits.push(amount.clone());
                }
                ProviderMode::ManualPrepaidCard => {
                    let provider = self.state.manual_provider.as_mut().ok_or_else(|| {
                        TreasuryError::Conflict("manual provider is not configured".to_string())
                    })?;
                    let snapshot = provider.balance_snapshot.as_mut().ok_or_else(|| {
                        TreasuryError::Conflict(
                            "manual balance snapshot is unavailable".to_string(),
                        )
                    })?;
                    snapshot.amount = snapshot.amount.checked_add(&amount)?;
                    snapshot.status = BalanceStatus::OwnerConfirmed;
                    snapshot.observed_at = now();
                }
            }
        }
        let kind = if verified {
            LedgerEventKind::IncomeVerified
        } else {
            LedgerEventKind::IncomeUnverified
        };
        self.append_ledger(kind, amount.clone(), None, &source, verified)?;
        if reinvested.minor > 0 {
            self.append_ledger(
                LedgerEventKind::OperatorTopUp,
                reinvested.clone(),
                None,
                "verified-income-reinvestment",
                true,
            )?;
        }
        self.state.processed_deposits.insert(external_reference.clone(), fingerprint);
        self.audit(
            "owner",
            "record_deposit",
            None,
            None,
            Some(if verified { "verified" } else { "unverified" }),
            json!({ "amount": amount, "source": source, "agent_id": agent_id, "reinvested": reinvested, "external_reference": external_reference }),
        )?;
        Ok(
            json!({ "recorded": true, "duplicate": false, "verified": verified, "spendable": reinvested.minor > 0, "reinvested": reinvested, "external_reference": external_reference }),
        )
    }

    fn owner_configure_receive(
        &mut self,
        actor: &Actor,
        method: String,
        address: String,
        memo_template: String,
    ) -> Result<Value> {
        Self::require_owner(actor)?;
        bounded(&method, "receive_method", 64)?;
        bounded(&address, "receive_address", 320)?;
        bounded(&memo_template, "memo_template", 160)?;
        if !method.eq_ignore_ascii_case("interac_e_transfer")
            && !method.eq_ignore_ascii_case("manual")
        {
            return Err(TreasuryError::Invalid(
                "initial release supports only manual or Interac e-Transfer receiving instructions"
                    .to_string(),
            ));
        }
        self.state.receive_instructions = Some(ReceiveInstructions {
            method,
            address,
            memo_template,
            public: true,
            configured_at: now(),
        });
        self.audit(
            "owner",
            "configure_receive_instructions",
            None,
            None,
            Some("allowed"),
            json!({ "public": true }),
        )?;
        Ok(json!({ "configured": true, "outgoing_transfers_supported": false }))
    }

    fn owner_configure_manual_provider(
        &mut self,
        actor: &Actor,
        configuration: ManualProviderConfiguration,
    ) -> Result<Value> {
        Self::require_owner(actor)?;
        let ManualProviderConfiguration {
            credential_reference,
            provider_kind,
            last_four,
            balance,
            balance_status,
            balance_ttl_secs,
        } = configuration;
        bounded(&credential_reference, "credential_reference", 256)?;
        bounded(&provider_kind, "provider_kind", 64)?;
        if redact_sensitive(&credential_reference) != credential_reference {
            return Err(TreasuryError::Invalid(
                "credential_reference appears to contain payment data".to_string(),
            ));
        }
        if self.state.intents.values().any(|intent| {
            !matches!(
                intent.state,
                TransactionState::Declined
                    | TransactionState::Failed
                    | TransactionState::Cancelled
                    | TransactionState::Refunded
            )
        }) {
            return Err(TreasuryError::Conflict(
                "provider cannot change while non-terminal purchase history exists".to_string(),
            ));
        }
        if balance.minor < 0 || balance_ttl_secs <= 0 || balance_ttl_secs > 24 * 60 * 60 {
            return Err(TreasuryError::Invalid(
                "manual balance and freshness TTL are invalid".to_string(),
            ));
        }
        if let Some(last_four) = &last_four
            && (last_four.len() != 4 || !last_four.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return Err(TreasuryError::Invalid(
                "last_four must contain exactly four digits".to_string(),
            ));
        }
        for policy in self.state.policies.values() {
            if balance.currency != policy.primary_currency
                || balance.minor > policy.max_treasury_size.minor
            {
                return Err(TreasuryError::Forbidden(
                    "manual balance is outside an active policy treasury envelope".to_string(),
                ));
            }
        }
        let mut provider = ManualPrepaidCardProvider::new(SecretReference {
            reference: credential_reference,
            provider_kind,
            last_four,
            persisted_secret: false,
        });
        let at = now();
        provider.balance_snapshot = Some(ManualBalanceSnapshot {
            amount: balance.clone(),
            status: balance_status.clone(),
            observed_at: at,
            source: "owner-provider-configuration".to_string(),
            expires_at: at + balance_ttl_secs,
        });
        self.state.provider_mode = ProviderMode::ManualPrepaidCard;
        self.state.manual_provider = Some(provider);
        self.audit(
            "owner",
            "configure_manual_provider",
            None,
            None,
            Some("allowed"),
            json!({
                "provider": "manual-prepaid-card",
                "balance_status": balance_status,
                "balance": balance,
                "expires_at": at + balance_ttl_secs,
                "credential_material_persisted": false,
            }),
        )?;
        Ok(json!({
            "provider": "manual-prepaid-card",
            "balance_status": balance_status,
            "balance": balance,
            "expires_at": at + balance_ttl_secs,
            "credential_material_persisted": false,
        }))
    }

    fn owner_list_audit(&self, actor: &Actor) -> Result<Value> {
        Self::require_owner(actor)?;
        Ok(json!({ "entries": self.state.audit, "chain_valid": self.verify_audit_chain().is_ok() }))
    }

    fn agent(&self, agent_id: &str) -> Result<&AgentRecord> {
        self.state
            .agents
            .get(agent_id)
            .ok_or_else(|| TreasuryError::NotFound(format!("agent {agent_id}")))
    }

    fn policy(&self, policy_id: &str) -> Result<&Policy> {
        self.state
            .policies
            .get(policy_id)
            .ok_or_else(|| TreasuryError::NotFound(format!("policy {policy_id}")))
    }

    fn intent_for_agent(&self, agent_id: &str, intent_id: &str) -> Result<&PurchaseIntent> {
        let intent = self
            .state
            .intents
            .get(intent_id)
            .ok_or_else(|| TreasuryError::NotFound(intent_id.to_string()))?;
        if intent.agent_id != agent_id {
            return Err(TreasuryError::Forbidden("intent belongs to another agent".to_string()));
        }
        Ok(intent)
    }

    fn usage(&self, agent_id: &str, currency: &str) -> Result<BudgetUsage> {
        let mut usage = BudgetUsage::zero(currency)?;
        let at = now();
        let session_id = self.agent(agent_id)?.broker_session_id.clone();
        usage.recent_transaction_count = self
            .state
            .intents
            .values()
            .filter(|intent| intent.agent_id == agent_id && intent.created_at >= at - 60)
            .count()
            .try_into()
            .unwrap_or(u32::MAX);
        for event in &self.state.ledger {
            if event.amount.currency != currency {
                continue;
            }
            let belongs = event
                .intent_id
                .as_ref()
                .and_then(|id| self.state.intents.get(id))
                .map(|intent| intent.agent_id == agent_id)
                .unwrap_or(event.intent_id.is_none());
            if !belongs {
                continue;
            }
            if event.kind == LedgerEventKind::SpendingSettled {
                usage.lifetime_amount = usage.lifetime_amount.checked_add(&event.amount)?;
                if event.at >= at - 24 * 60 * 60 {
                    usage.rolling_24h_amount =
                        usage.rolling_24h_amount.checked_add(&event.amount)?;
                }
                if let Some(intent_id) = &event.intent_id
                    && let Some(intent) = self.state.intents.get(intent_id)
                    && intent.broker_session_id == session_id
                {
                    usage.session_amount = usage.session_amount.checked_add(&event.amount)?;
                }
            }
        }
        for intent in self.state.intents.values().filter(|intent| intent.agent_id == agent_id) {
            if matches!(
                intent.state,
                TransactionState::FundsReserved
                    | TransactionState::Executing
                    | TransactionState::ProviderPending
                    | TransactionState::Unknown
                    | TransactionState::ReconciliationRequired
            ) {
                usage.reserved_amount =
                    usage.reserved_amount.checked_add(&intent.request.amount)?;
                usage.lifetime_amount =
                    usage.lifetime_amount.checked_add(&intent.request.amount)?;
                usage.rolling_24h_amount =
                    usage.rolling_24h_amount.checked_add(&intent.request.amount)?;
                if intent.broker_session_id == session_id {
                    usage.session_amount =
                        usage.session_amount.checked_add(&intent.request.amount)?;
                }
            }
        }
        Ok(usage)
    }

    fn enforce_authority(
        &self,
        decision: &mut PolicyDecision,
        request: &PurchaseRequest,
        usage: &BudgetUsage,
        policy: &Policy,
    ) -> Result<()> {
        if request.amount.currency != policy.primary_currency {
            return Ok(());
        }
        let ledger = self.ledger_snapshot(&policy.primary_currency)?;
        if ledger.available_authority.minor < request.amount.minor {
            decision.allowed = false;
            decision.requires_approval = false;
            decision.reasons.push("available policy authority is insufficient".to_string());
        }
        let provider_balance = self.provider_available_balance()?;
        if provider_balance.minor < request.amount.minor {
            decision.allowed = false;
            decision.requires_approval = false;
            decision.reasons.push("provider balance is insufficient".to_string());
        }
        let configured_balance = match self.state.provider_mode {
            ProviderMode::Simulated => &self.state.provider.balance,
            ProviderMode::ManualPrepaidCard => {
                &self
                    .state
                    .manual_provider
                    .as_ref()
                    .and_then(|provider| provider.balance_snapshot.as_ref())
                    .ok_or_else(|| {
                        TreasuryError::Conflict(
                            "manual balance snapshot is unavailable".to_string(),
                        )
                    })?
                    .amount
            }
        };
        if configured_balance.minor > policy.max_treasury_size.minor {
            decision.allowed = false;
            decision.requires_approval = false;
            decision.reasons.push("provider balance exceeds maximum treasury size".to_string());
        }
        if usage.reserved_amount.checked_add(&request.amount)?.minor
            > policy.absolute_exposure_ceiling.minor
        {
            decision.allowed = false;
            decision.requires_approval = false;
            decision.reasons.push("absolute exposure ceiling exceeded".to_string());
        }
        Ok(())
    }

    fn append_ledger(
        &mut self,
        kind: LedgerEventKind,
        amount: Money,
        intent_id: Option<String>,
        source: &str,
        verified: bool,
    ) -> Result<()> {
        self.state.ledger.push(LedgerEvent {
            id: new_id("ledger"),
            at: now(),
            kind,
            amount,
            intent_id,
            source: source.to_string(),
            verified,
        });
        Ok(())
    }

    fn record_settled(&mut self, intent: &PurchaseIntent) -> Result<()> {
        if !self.state.ledger.iter().any(|event| {
            event.intent_id.as_deref() == Some(&intent.id)
                && event.kind == LedgerEventKind::HoldReleased
        }) {
            self.append_ledger(
                LedgerEventKind::HoldReleased,
                intent.request.amount.clone(),
                Some(intent.id.clone()),
                "settlement",
                true,
            )?;
        }
        if self.state.ledger.iter().any(|event| {
            event.intent_id.as_deref() == Some(&intent.id)
                && event.kind == LedgerEventKind::SpendingSettled
        }) {
            return Ok(());
        }
        self.append_ledger(
            LedgerEventKind::SpendingSettled,
            intent.request.amount.clone(),
            Some(intent.id.clone()),
            "simulated-provider",
            true,
        )
    }

    pub fn ledger_snapshot(&self, currency: &str) -> Result<LedgerSnapshot> {
        let zero = Money::zero(currency)?;
        let mut snapshot = LedgerSnapshot {
            currency: currency.to_string(),
            owner_capital: zero.clone(),
            verified_income: zero.clone(),
            unverified_income: zero.clone(),
            operator_topups: zero.clone(),
            settled_spending: zero.clone(),
            refunds: zero.clone(),
            reserved_amount: zero.clone(),
            available_authority: zero,
        };
        for event in &self.state.ledger {
            if event.amount.currency != currency {
                continue;
            }
            match event.kind {
                LedgerEventKind::OwnerCapital => {
                    snapshot.owner_capital = snapshot.owner_capital.checked_add(&event.amount)?
                }
                LedgerEventKind::IncomeVerified => {
                    snapshot.verified_income =
                        snapshot.verified_income.checked_add(&event.amount)?
                }
                LedgerEventKind::IncomeUnverified => {
                    snapshot.unverified_income =
                        snapshot.unverified_income.checked_add(&event.amount)?
                }
                LedgerEventKind::OperatorTopUp => {
                    snapshot.operator_topups =
                        snapshot.operator_topups.checked_add(&event.amount)?
                }
                LedgerEventKind::SpendingSettled => {
                    snapshot.settled_spending =
                        snapshot.settled_spending.checked_add(&event.amount)?
                }
                LedgerEventKind::Refund => {
                    snapshot.refunds = snapshot.refunds.checked_add(&event.amount)?
                }
                _ => {}
            }
        }
        for intent in self.state.intents.values() {
            if intent.request.amount.currency == currency
                && matches!(
                    intent.state,
                    TransactionState::FundsReserved
                        | TransactionState::Executing
                        | TransactionState::ProviderPending
                        | TransactionState::Unknown
                        | TransactionState::ReconciliationRequired
                )
            {
                snapshot.reserved_amount =
                    snapshot.reserved_amount.checked_add(&intent.request.amount)?;
            }
        }
        let credits = snapshot
            .owner_capital
            .checked_add(&snapshot.operator_topups)?
            .checked_add(&snapshot.refunds)?;
        let debits = snapshot.settled_spending.checked_add(&snapshot.reserved_amount)?;
        let available = credits.checked_sub(&debits)?;
        if available.minor < 0 {
            return Err(TreasuryError::Conflict(
                "ledger authority invariant would become negative".to_string(),
            ));
        }
        snapshot.available_authority = available;
        Ok(snapshot)
    }

    fn sanitized_intent(&self, intent: &PurchaseIntent) -> Value {
        json!({
            "id": intent.id,
            "agent_id": intent.agent_id,
            "state": intent.state,
            "amount": intent.request.amount,
            "merchant_domain": intent.request.merchant_domain,
            "category": intent.request.category,
            "fulfillment_profile": intent.request.fulfillment_profile,
            "decision": intent.decision,
            "policy_version": intent.policy_version,
            "created_at": intent.created_at,
            "updated_at": intent.updated_at,
            "provider_reference": intent.provider_reference,
            "receipt_hash": intent.receipt_hash,
            "last_error": intent.last_error,
        })
    }

    fn receipt(&self, intent: &PurchaseIntent) -> Result<Receipt> {
        Ok(Receipt {
            intent_id: intent.id.clone(),
            merchant_domain: intent.request.merchant_domain.clone(),
            amount: intent.request.amount.clone(),
            status: intent.state.clone(),
            provider_reference: intent.provider_reference.clone(),
            issued_at: now(),
            personal_information_redacted: true,
        })
    }

    fn receipt_hash(receipt: &Receipt) -> Result<String> {
        let encoded = serde_json::to_vec(receipt)?;
        let mut digest = Sha256::new();
        digest.update(encoded);
        Ok(hex::encode(digest.finalize()))
    }

    fn audit(
        &mut self,
        actor: &str,
        action: &str,
        intent_id: Option<&str>,
        policy_version: Option<u64>,
        decision: Option<&str>,
        details: Value,
    ) -> Result<()> {
        self.state.generation = self
            .state
            .generation
            .checked_add(1)
            .ok_or_else(|| TreasuryError::Conflict("state generation overflow".to_string()))?;
        let details = redact_json_value(details);
        let sequence = self.state.audit.len() as u64 + 1;
        let previous_hash =
            self.state.audit.last().map(|entry| entry.hash.clone()).unwrap_or_default();
        let unsigned = json!({ "sequence": sequence, "at": now(), "actor": actor, "action": action, "intent_id": intent_id, "policy_version": policy_version, "decision": decision, "details": details, "previous_hash": previous_hash });
        let hash = hmac_hash(&self.audit_key, &unsigned);
        self.state.audit.push(AuditEntry {
            sequence,
            at: unsigned["at"].as_i64().unwrap_or_default(),
            actor: actor.to_string(),
            action: action.to_string(),
            intent_id: intent_id.map(str::to_string),
            policy_version,
            decision: decision.map(str::to_string),
            details: unsigned["details"].clone(),
            previous_hash,
            hash,
        });
        Ok(())
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter().zip(right).fold(0_u8, |difference, (a, b)| difference | (a ^ b)) == 0
}

fn ensure_private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(TreasuryError::Invalid(
                "treasury data directory must be a real directory".to_string(),
            ));
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn verify_private_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(TreasuryError::Invalid(format!(
            "{} must be a regular file and not a symlink",
            path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(TreasuryError::Invalid(format!(
                "{} permissions must deny group and other access",
                path.display()
            )));
        }
    }
    Ok(())
}

fn read_private_file(path: &Path) -> Result<Vec<u8>> {
    verify_private_file(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn write_new_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    verify_private_file(path)?;
    Ok(())
}

fn write_atomic_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| TreasuryError::Invalid("state path has no parent".to_string()))?;
    let temp = parent.join(format!(".state-{}.tmp", random_hex(12)));
    write_new_private_file(&temp, bytes)?;
    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    verify_private_file(path)?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    fs::File::open(path)?.sync_all()?;
    Ok(())
}

pub fn canonicalize_domain(input: &str) -> Result<String> {
    bounded(input, "domain", 320)?;
    let candidate = if input.contains("://") {
        let url =
            Url::parse(input).map_err(|_| TreasuryError::Invalid("malformed URL".to_string()))?;
        if url.scheme() != "https"
            || url.username() != ""
            || url.password().is_some()
            || url.port().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/" && !url.path().is_empty()
        {
            return Err(TreasuryError::Invalid(
                "merchant URLs must be HTTPS origin URLs without credentials, ports, or paths"
                    .to_string(),
            ));
        }
        url.host_str()
            .ok_or_else(|| TreasuryError::Invalid("URL has no host".to_string()))?
            .to_string()
    } else {
        input.trim_end_matches('.').to_string()
    };
    let ascii = domain_to_ascii(&candidate)
        .map_err(|_| TreasuryError::Invalid("invalid internationalized domain".to_string()))?;
    let ascii = ascii.trim_end_matches('.').to_ascii_lowercase();
    if ascii.is_empty()
        || ascii.contains('/')
        || ascii.contains('@')
        || ascii.contains('?')
        || ascii.contains('#')
        || ascii == "localhost"
        || ascii.ends_with(".localhost")
    {
        return Err(TreasuryError::Invalid("local or malformed domain is not allowed".to_string()));
    }
    if let Ok(ip) = ascii.parse::<IpAddr>() {
        validate_resolved_ip(ip)?;
    }
    if ascii
        .split('.')
        .any(|label| label.is_empty() || label.starts_with('-') || label.ends_with('-'))
    {
        return Err(TreasuryError::Invalid("invalid DNS label".to_string()));
    }
    Ok(ascii)
}

pub fn validate_resolved_ip(ip: IpAddr) -> Result<()> {
    if is_non_public_ip(ip) {
        return Err(TreasuryError::Invalid(
            "private, loopback, link-local, multicast, and metadata IPs are blocked".to_string(),
        ));
    }
    Ok(())
}

fn is_non_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(value) => {
            let [a, b, c, _] = value.octets();
            a == 0
                || a == 10
                || a == 100 && (64..=127).contains(&b)
                || a == 127
                || a == 169 && b == 254
                || a == 172 && (16..=31).contains(&b)
                || a == 192 && b == 0 && c == 0
                || a == 192 && b == 0 && c == 2
                || a == 192 && b == 168
                || a == 198 && (b == 18 || b == 19)
                || a == 198 && b == 51 && c == 100
                || a == 203 && b == 0 && c == 113
                || a >= 224
        }
        IpAddr::V6(value) => {
            if let Some(mapped) = value.to_ipv4_mapped() {
                return is_non_public_ip(IpAddr::V4(mapped));
            }
            let first = value.segments()[0];
            value.is_loopback()
                || value.is_unspecified()
                || value.is_multicast()
                || first & 0xfe00 == 0xfc00
                || first & 0xffc0 == 0xfe80
                || first & 0xffc0 == 0xfec0
                || value.segments()[0..2] == [0x2001, 0x0db8]
        }
    }
}

pub fn validate_https_url(input: &str) -> Result<String> {
    let url = Url::parse(input)
        .map_err(|_| TreasuryError::Invalid("malformed redirect URL".to_string()))?;
    if url.scheme() != "https"
        || url.username() != ""
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err(TreasuryError::Invalid(
            "redirects must be HTTPS without credentials or explicit ports".to_string(),
        ));
    }
    let host =
        url.host_str().ok_or_else(|| TreasuryError::Invalid("redirect has no host".to_string()))?;
    let canonical = canonicalize_domain(host)?;
    Ok(canonical)
}

static PAN_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:[0-9][ -]?){12,18}[0-9]\b").expect("PAN redaction regex is valid")
});
static CVV_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(cvv2?|cvc2?|security[ _-]?code)([[:space:]]*[:=]?[[:space:]]*)[0-9]{3,4}\b")
        .expect("CVV redaction regex is valid")
});

pub fn redact_sensitive(input: &str) -> String {
    let without_pan = PAN_PATTERN.replace_all(input, "[REDACTED_PAN]");
    CVV_PATTERN.replace_all(&without_pan, "$1$2[REDACTED_CVV]").into_owned()
}

fn validate_provider_reference(input: &str) -> Result<String> {
    bounded(input, "provider_reference", 128)?;
    if redact_sensitive(input) != input {
        return Err(TreasuryError::Invalid(
            "provider reference appears to contain payment data".to_string(),
        ));
    }
    if !input.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(TreasuryError::Invalid(
            "provider reference contains unsupported characters".to_string(),
        ));
    }
    Ok(input.to_string())
}

fn sanitize_provider_reference(input: &str) -> String {
    validate_provider_reference(input).unwrap_or_else(|_| {
        let mut digest = Sha256::new();
        digest.update(input.as_bytes());
        format!("provider_ref_{}", &hex::encode(digest.finalize())[..24])
    })
}

fn opaque_reference(kind: &str, input: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(format!("agent-treasury {kind} reference v1\0").as_bytes());
    digest.update(input.as_bytes());
    format!("{kind}_ref_{}", &hex::encode(digest.finalize())[..32])
}

fn redact_json_value(value: Value) -> Value {
    match value {
        Value::String(value) => Value::String(redact_sensitive(&value)),
        Value::Array(values) => Value::Array(values.into_iter().map(redact_json_value).collect()),
        Value::Object(values) => Value::Object(
            values.into_iter().map(|(key, value)| (key, redact_json_value(value))).collect(),
        ),
        value => value,
    }
}

pub trait SecretProvider {
    fn kind(&self) -> &'static str;
    fn reference(&self) -> &str;
    fn last_four(&self) -> Option<&str>;
    fn fetch_for_owner_operation(&mut self, operation_id: &str) -> Result<VolatileSecret>;
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretReference {
    pub reference: String,
    pub provider_kind: String,
    pub last_four: Option<String>,
    pub persisted_secret: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolatileSecret {
    bytes: Vec<u8>,
}

impl VolatileSecret {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl Drop for VolatileSecret {
    fn drop(&mut self) {
        for byte in &mut self.bytes {
            *byte = 0;
        }
    }
}

pub struct SimulatedSecretProvider {
    reference: String,
    last_four: String,
    canary: Vec<u8>,
}

impl SimulatedSecretProvider {
    pub fn new(canary_pan: &str, canary_cvv: &str) -> Self {
        let mut canary = canary_pan.as_bytes().to_vec();
        canary.extend_from_slice(canary_cvv.as_bytes());
        Self {
            reference: "simulated-card".to_string(),
            last_four: canary_pan.chars().rev().take(4).collect::<String>().chars().rev().collect(),
            canary,
        }
    }
}

impl SecretProvider for SimulatedSecretProvider {
    fn kind(&self) -> &'static str {
        "simulated_test_provider"
    }
    fn reference(&self) -> &str {
        &self.reference
    }
    fn last_four(&self) -> Option<&str> {
        Some(&self.last_four)
    }
    fn fetch_for_owner_operation(&mut self, _operation_id: &str) -> Result<VolatileSecret> {
        Ok(VolatileSecret::new(self.canary.clone()))
    }
}

pub fn read_rpc_lines<R: std::io::Read>(reader: R) -> impl Iterator<Item = Result<RpcRequest>> {
    BufReader::new(reader).lines().map(|line| match line {
        Ok(line) => serde_json::from_str(&line).map_err(TreasuryError::from),
        Err(error) => Err(TreasuryError::Io(error)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn request(key: &str, amount: i64) -> PurchaseRequest {
        PurchaseRequest {
            idempotency_key: key.to_string(),
            amount: Money::positive(amount, "CAD").unwrap(),
            final_total: Money::positive(amount, "CAD").unwrap(),
            merchant_domain: "merchant.example.test".to_string(),
            category: "software".to_string(),
            recurring: false,
            trial_auto_renew: false,
            stored_card: false,
            tip_minor: 0,
            preauthorization: false,
            installments: false,
            fulfillment_profile: "digital-email".to_string(),
            payment_form: PaymentFormTrust::HostedFields,
            redirect_chain: vec!["https://merchant.example.test/checkout".to_string()],
            attempts: 1,
            session_id: "test-session".to_string(),
            scenario: SimulatedScenario::Normal,
        }
    }

    fn create_agent(
        treasury: &mut Treasury,
        owner_token: &str,
        policy: Policy,
        mode: AutonomyMode,
    ) -> (String, String) {
        let created = treasury
            .handle(
                owner_token,
                Request::OwnerCreateAgent {
                    name: "test-agent".to_string(),
                    policy,
                    mode,
                    ttl_secs: 3600,
                },
            )
            .unwrap();
        (
            created["agent_id"].as_str().unwrap().to_string(),
            created["capability_token"].as_str().unwrap().to_string(),
        )
    }

    #[test]
    fn money_rejects_currency_mismatch_and_overflow() {
        let cad = Money::new(i64::MAX, "CAD").unwrap();
        let usd = Money::new(1, "USD").unwrap();
        assert!(cad.checked_add(&usd).is_err());
        assert!(cad.checked_add(&Money::new(1, "CAD").unwrap()).is_err());
    }

    #[test]
    fn domain_validation_blocks_local_and_normalizes() {
        assert_eq!(canonicalize_domain("Merchant.Example.Test.").unwrap(), "merchant.example.test");
        assert!(canonicalize_domain("http://merchant.example.test").is_err());
        assert!(
            canonicalize_domain("https://merchant.example.test/?next=https://evil.test").is_err()
        );
        assert!(canonicalize_domain("localhost").is_err());
        assert!(canonicalize_domain("127.0.0.1").is_err());
        assert!(validate_resolved_ip("127.0.0.1".parse().unwrap()).is_err());
        assert!(validate_resolved_ip("8.8.8.8".parse().unwrap()).is_ok());
        assert!(validate_https_url("https://merchant.example.test/checkout").is_ok());
        assert!(validate_https_url("file:///tmp/card").is_err());
    }

    #[test]
    fn state_machine_rejects_retry_from_unknown() {
        assert!(
            TransactionState::Unknown.can_transition(&TransactionState::ReconciliationRequired)
        );
        assert!(!TransactionState::Unknown.can_transition(&TransactionState::Executing));
    }

    #[test]
    fn unverified_income_is_not_spendable() {
        let bootstrap =
            Treasury::bootstrap("owner", Money::positive(1_000, "CAD").unwrap()).unwrap();
        let mut treasury = bootstrap.treasury;
        let before = treasury.state.provider.balance.clone();
        treasury
            .owner_record_deposit(
                &Actor::Owner,
                Money::positive(500, "CAD").unwrap(),
                "email-notification".to_string(),
                false,
                None,
                "notification-1".to_string(),
            )
            .unwrap();
        assert_eq!(treasury.state.provider.balance, before);
        assert!(
            treasury
                .state
                .ledger
                .iter()
                .any(|event| event.kind == LedgerEventKind::IncomeUnverified && !event.verified)
        );
    }

    #[test]
    fn capability_cannot_run_owner_operation() {
        let bootstrap =
            Treasury::bootstrap("owner", Money::positive(10_000, "CAD").unwrap()).unwrap();
        let mut treasury = bootstrap.treasury;
        let policy = Policy::conservative_demo().unwrap();
        let owner_response = treasury
            .handle(
                &bootstrap.owner_token,
                Request::OwnerCreateAgent {
                    name: "agent".to_string(),
                    policy,
                    mode: AutonomyMode::BoundedAutonomous,
                    ttl_secs: 3600,
                },
            )
            .unwrap();
        let agent_token = owner_response["capability_token"].as_str().unwrap();
        assert!(
            treasury.handle(agent_token, Request::OwnerSetEmergencyStop { stopped: true }).is_err()
        );
    }

    #[test]
    fn owner_can_pause_and_revoke_agent_capability() {
        let bootstrap =
            Treasury::bootstrap("owner", Money::positive(10_000, "CAD").unwrap()).unwrap();
        let mut treasury = bootstrap.treasury;
        let created = treasury
            .handle(
                &bootstrap.owner_token,
                Request::OwnerCreateAgent {
                    name: "lifecycle-agent".to_string(),
                    policy: Policy::conservative_demo().unwrap(),
                    mode: AutonomyMode::BoundedAutonomous,
                    ttl_secs: 3600,
                },
            )
            .unwrap();
        let agent_id = created["agent_id"].as_str().unwrap().to_string();
        let agent_token = created["capability_token"].as_str().unwrap();
        treasury
            .handle(
                &bootstrap.owner_token,
                Request::OwnerSetAgentMode {
                    agent_id: agent_id.clone(),
                    mode: AutonomyMode::Observe,
                },
            )
            .unwrap();
        assert_eq!(treasury.handle(agent_token, Request::GetStatus).unwrap()["mode"], "observe");
        treasury.handle(&bootstrap.owner_token, Request::OwnerRevokeAgent { agent_id }).unwrap();
        assert!(treasury.handle(agent_token, Request::GetStatus).is_err());
    }

    #[test]
    fn duplicate_idempotency_does_not_create_second_intent() {
        let bootstrap =
            Treasury::bootstrap("owner", Money::positive(10_000, "CAD").unwrap()).unwrap();
        let mut treasury = bootstrap.treasury;
        let policy = Policy::conservative_demo().unwrap();
        let owner_response = treasury
            .handle(
                &bootstrap.owner_token,
                Request::OwnerCreateAgent {
                    name: "agent".to_string(),
                    policy,
                    mode: AutonomyMode::BoundedAutonomous,
                    ttl_secs: 3600,
                },
            )
            .unwrap();
        let token = owner_response["capability_token"].as_str().unwrap();
        let first = treasury
            .handle(token, Request::CreatePurchaseIntent { request: request("same", 500) })
            .unwrap();
        let second = treasury
            .handle(token, Request::CreatePurchaseIntent { request: request("same", 500) })
            .unwrap();
        assert_eq!(first["id"], second["id"]);
        assert_eq!(treasury.state.intents.len(), 1);
    }

    #[test]
    fn timeout_after_submit_cannot_be_retried() {
        let bootstrap =
            Treasury::bootstrap("owner", Money::positive(10_000, "CAD").unwrap()).unwrap();
        let mut treasury = bootstrap.treasury;
        let owner_response = treasury
            .handle(
                &bootstrap.owner_token,
                Request::OwnerCreateAgent {
                    name: "agent".to_string(),
                    policy: Policy::conservative_demo().unwrap(),
                    mode: AutonomyMode::BoundedAutonomous,
                    ttl_secs: 3600,
                },
            )
            .unwrap();
        let token = owner_response["capability_token"].as_str().unwrap();
        let mut purchase = request("timeout", 500);
        purchase.scenario = SimulatedScenario::TimeoutAfterSubmit;
        let intent =
            treasury.handle(token, Request::CreatePurchaseIntent { request: purchase }).unwrap();
        let result = treasury
            .handle(
                token,
                Request::ExecutePurchaseIntent {
                    intent_id: intent["id"].as_str().unwrap().to_string(),
                },
            )
            .unwrap();
        assert_eq!(result["status"], "unknown");
        assert!(
            treasury
                .handle(
                    token,
                    Request::ExecutePurchaseIntent {
                        intent_id: intent["id"].as_str().unwrap().to_string()
                    }
                )
                .is_err()
        );
    }

    #[test]
    fn verified_income_increases_provider_balance_but_unverified_income_does_not() {
        let bootstrap =
            Treasury::bootstrap("owner", Money::positive(1_000, "CAD").unwrap()).unwrap();
        let mut treasury = bootstrap.treasury;
        treasury
            .owner_record_deposit(
                &Actor::Owner,
                Money::positive(500, "CAD").unwrap(),
                "spoofed-email".to_string(),
                false,
                None,
                "spoofed-email-1".to_string(),
            )
            .unwrap();
        assert_eq!(treasury.state.provider.balance.minor, 1_000);
        let created = treasury
            .handle(
                &bootstrap.owner_token,
                Request::OwnerCreateAgent {
                    name: "deposit-agent".to_string(),
                    policy: Policy::conservative_demo().unwrap(),
                    mode: AutonomyMode::ApprovalRequired,
                    ttl_secs: 3600,
                },
            )
            .unwrap();
        let agent_id = created["agent_id"].as_str().unwrap().to_string();
        treasury
            .owner_record_deposit(
                &Actor::Owner,
                Money::positive(500, "CAD").unwrap(),
                "owner-reconciled-provider-reference".to_string(),
                true,
                Some(agent_id),
                "provider-deposit-1".to_string(),
            )
            .unwrap();
        assert_eq!(treasury.state.provider.balance.minor, 1_500);
        let ledger = treasury.ledger_snapshot("CAD").unwrap();
        assert_eq!(ledger.unverified_income.minor, 500);
        assert_eq!(ledger.verified_income.minor, 500);
        assert_eq!(ledger.available_authority.minor, 1_000);
    }

    #[test]
    fn verified_income_reinvests_only_the_configured_integer_ratio() {
        let bootstrap =
            Treasury::bootstrap("owner", Money::positive(1_000, "CAD").unwrap()).unwrap();
        let mut treasury = bootstrap.treasury;
        let mut policy = Policy::conservative_demo().unwrap();
        policy.reinvestment_ratio_bps = 5_000;
        let created = treasury
            .handle(
                &bootstrap.owner_token,
                Request::OwnerCreateAgent {
                    name: "reinvest-agent".to_string(),
                    policy,
                    mode: AutonomyMode::BoundedAutonomous,
                    ttl_secs: 3600,
                },
            )
            .unwrap();
        let agent_id = created["agent_id"].as_str().unwrap().to_string();
        treasury
            .owner_record_deposit(
                &Actor::Owner,
                Money::positive(500, "CAD").unwrap(),
                "owner-provider-reconciliation".to_string(),
                true,
                Some(agent_id),
                "provider-deposit-2".to_string(),
            )
            .unwrap();
        let ledger = treasury.ledger_snapshot("CAD").unwrap();
        assert_eq!(ledger.verified_income.minor, 500);
        assert_eq!(ledger.operator_topups.minor, 250);
        assert_eq!(ledger.available_authority.minor, 1_250);
    }

    #[test]
    fn approval_mode_needs_owner_action_before_execution() {
        let bootstrap =
            Treasury::bootstrap("owner", Money::positive(10_000, "CAD").unwrap()).unwrap();
        let mut treasury = bootstrap.treasury;
        let owner_response = treasury
            .handle(
                &bootstrap.owner_token,
                Request::OwnerCreateAgent {
                    name: "approval-agent".to_string(),
                    policy: Policy::conservative_demo().unwrap(),
                    mode: AutonomyMode::ApprovalRequired,
                    ttl_secs: 3600,
                },
            )
            .unwrap();
        let token = owner_response["capability_token"].as_str().unwrap();
        let intent = treasury
            .handle(token, Request::CreatePurchaseIntent { request: request("approval", 500) })
            .unwrap();
        assert_eq!(intent["state"], "approval_required");
        assert!(
            treasury
                .handle(
                    token,
                    Request::ExecutePurchaseIntent {
                        intent_id: intent["id"].as_str().unwrap().to_string(),
                    },
                )
                .is_err()
        );
        treasury
            .handle(
                &bootstrap.owner_token,
                Request::OwnerApproveIntent {
                    intent_id: intent["id"].as_str().unwrap().to_string(),
                },
            )
            .unwrap();
        let executed = treasury
            .handle(
                token,
                Request::ExecutePurchaseIntent {
                    intent_id: intent["id"].as_str().unwrap().to_string(),
                },
            )
            .unwrap();
        assert_eq!(executed["status"], "settled");
    }

    #[test]
    fn pending_provider_hold_is_reconciled_once() {
        let bootstrap =
            Treasury::bootstrap("owner", Money::positive(10_000, "CAD").unwrap()).unwrap();
        let mut treasury = bootstrap.treasury;
        let owner_response = treasury
            .handle(
                &bootstrap.owner_token,
                Request::OwnerCreateAgent {
                    name: "pending-agent".to_string(),
                    policy: Policy::conservative_demo().unwrap(),
                    mode: AutonomyMode::BoundedAutonomous,
                    ttl_secs: 3600,
                },
            )
            .unwrap();
        let token = owner_response["capability_token"].as_str().unwrap();
        let mut purchase = request("pending", 500);
        purchase.scenario = SimulatedScenario::DelayedSettlement;
        let intent =
            treasury.handle(token, Request::CreatePurchaseIntent { request: purchase }).unwrap();
        let pending = treasury
            .handle(
                token,
                Request::ExecutePurchaseIntent {
                    intent_id: intent["id"].as_str().unwrap().to_string(),
                },
            )
            .unwrap();
        assert_eq!(pending["status"], "provider_pending");
        assert_eq!(treasury.state.provider.holds.len(), 1);
        treasury
            .handle(
                &bootstrap.owner_token,
                Request::OwnerReconcile {
                    intent_id: intent["id"].as_str().unwrap().to_string(),
                    outcome: ReconciliationOutcome::Settled,
                    provider_reference: None,
                },
            )
            .unwrap();
        assert!(treasury.state.provider.holds.is_empty());
        assert_eq!(treasury.state.provider.charges.len(), 1);
        assert_eq!(treasury.ledger_snapshot("CAD").unwrap().reserved_amount.minor, 0);
    }

    #[test]
    fn pending_reservations_consume_session_rolling_and_lifetime_budgets() {
        let bootstrap =
            Treasury::bootstrap("owner", Money::positive(10_000, "CAD").unwrap()).unwrap();
        let mut treasury = bootstrap.treasury;
        let mut policy = Policy::conservative_demo().unwrap();
        policy.max_per_transaction = Money::positive(600, "CAD").unwrap();
        policy.max_per_session = Money::positive(1_000, "CAD").unwrap();
        policy.max_rolling_24h = Money::positive(1_000, "CAD").unwrap();
        policy.max_lifetime = Money::positive(1_000, "CAD").unwrap();
        let (_, token) = create_agent(
            &mut treasury,
            &bootstrap.owner_token,
            policy,
            AutonomyMode::BoundedAutonomous,
        );
        let mut first = request("reserved-first", 600);
        first.session_id = "caller-session-a".to_string();
        first.scenario = SimulatedScenario::DelayedSettlement;
        let first_intent =
            treasury.handle(&token, Request::CreatePurchaseIntent { request: first }).unwrap();
        treasury
            .handle(
                &token,
                Request::ExecutePurchaseIntent {
                    intent_id: first_intent["id"].as_str().unwrap().to_string(),
                },
            )
            .unwrap();
        let mut second = request("reserved-second", 600);
        second.session_id = "caller-session-b".to_string();
        let denied =
            treasury.handle(&token, Request::CreatePurchaseIntent { request: second }).unwrap();
        assert_eq!(denied["state"], "failed");
        let reasons = denied["decision"]["reasons"].as_array().unwrap();
        assert!(reasons.iter().any(|reason| reason == "per-session limit exceeded"));
        assert!(reasons.iter().any(|reason| reason == "rolling 24-hour limit exceeded"));
        assert!(reasons.iter().any(|reason| reason == "lifetime limit exceeded"));
    }

    #[test]
    fn failed_intent_does_not_approve_a_new_merchant() {
        let bootstrap =
            Treasury::bootstrap("owner", Money::positive(10_000, "CAD").unwrap()).unwrap();
        let mut treasury = bootstrap.treasury;
        let mut policy = Policy::conservative_demo().unwrap();
        policy.allowed_merchants.clear();
        let (_, token) = create_agent(
            &mut treasury,
            &bootstrap.owner_token,
            policy,
            AutonomyMode::BoundedAutonomous,
        );
        let mut denied = request("new-merchant-denied", 3_000);
        denied.merchant_domain = "new.example.test".to_string();
        denied.redirect_chain = vec!["https://new.example.test/checkout".to_string()];
        assert_eq!(
            treasury.handle(&token, Request::CreatePurchaseIntent { request: denied }).unwrap()["state"],
            "failed"
        );
        let mut valid = request("new-merchant-valid", 500);
        valid.merchant_domain = "new.example.test".to_string();
        valid.redirect_chain = vec!["https://new.example.test/checkout".to_string()];
        assert_eq!(
            treasury.handle(&token, Request::CreatePurchaseIntent { request: valid }).unwrap()["state"],
            "approval_required"
        );
    }

    #[test]
    fn unapproved_cross_origin_redirect_is_denied() {
        let bootstrap =
            Treasury::bootstrap("owner", Money::positive(10_000, "CAD").unwrap()).unwrap();
        let mut treasury = bootstrap.treasury;
        let (_, token) = create_agent(
            &mut treasury,
            &bootstrap.owner_token,
            Policy::conservative_demo().unwrap(),
            AutonomyMode::BoundedAutonomous,
        );
        let mut purchase = request("cross-origin", 500);
        purchase.redirect_chain = vec!["https://attacker.example.test/pay".to_string()];
        let denied =
            treasury.handle(&token, Request::CreatePurchaseIntent { request: purchase }).unwrap();
        assert_eq!(denied["state"], "failed");
        assert!(
            denied["decision"]["reasons"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reason| reason.as_str().unwrap().contains("redirect domain"))
        );
    }

    #[test]
    fn verified_deposit_reference_is_idempotent() {
        let bootstrap =
            Treasury::bootstrap("owner", Money::positive(1_000, "CAD").unwrap()).unwrap();
        let mut treasury = bootstrap.treasury;
        let mut policy = Policy::conservative_demo().unwrap();
        policy.reinvestment_ratio_bps = 5_000;
        let (agent_id, _) = create_agent(
            &mut treasury,
            &bootstrap.owner_token,
            policy,
            AutonomyMode::ApprovalRequired,
        );
        let first = treasury
            .owner_record_deposit(
                &Actor::Owner,
                Money::positive(500, "CAD").unwrap(),
                "owner-reconciled".to_string(),
                true,
                Some(agent_id.clone()),
                "external-deposit-42".to_string(),
            )
            .unwrap();
        let second = treasury
            .owner_record_deposit(
                &Actor::Owner,
                Money::positive(500, "CAD").unwrap(),
                "owner-reconciled".to_string(),
                true,
                Some(agent_id),
                "external-deposit-42".to_string(),
            )
            .unwrap();
        assert_eq!(first["duplicate"], false);
        assert_eq!(second["duplicate"], true);
        assert_eq!(treasury.state.provider.balance.minor, 1_500);
        assert_eq!(treasury.ledger_snapshot("CAD").unwrap().operator_topups.minor, 250);
    }

    #[test]
    fn persisted_state_mac_rejects_security_state_and_audit_tail_tampering() {
        let directory = tempfile::tempdir().unwrap();
        let bootstrap =
            Treasury::bootstrap("owner", Money::positive(1_000, "CAD").unwrap()).unwrap();
        bootstrap.treasury.save_to(directory.path()).unwrap();
        let state_path = directory.path().join(STATE_FILE);
        let mut envelope: Value = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
        envelope["state"]["emergency_stop"] = json!(true);
        envelope["state"]["audit"] = json!([]);
        fs::write(&state_path, serde_json::to_vec_pretty(&envelope).unwrap()).unwrap();
        assert!(Treasury::load_from(directory.path()).is_err());
    }

    #[test]
    fn interrupted_persisted_execution_recovers_unknown_without_retry() {
        let directory = tempfile::tempdir().unwrap();
        let bootstrap =
            Treasury::bootstrap("owner", Money::positive(10_000, "CAD").unwrap()).unwrap();
        let mut treasury = bootstrap.treasury;
        let (_, token) = create_agent(
            &mut treasury,
            &bootstrap.owner_token,
            Policy::conservative_demo().unwrap(),
            AutonomyMode::BoundedAutonomous,
        );
        let intent = treasury
            .handle(
                &token,
                Request::CreatePurchaseIntent { request: request("crash-before-provider", 500) },
            )
            .unwrap();
        let actor = treasury.authenticate(&token).unwrap();
        treasury.prepare_intent_execution(&actor, intent["id"].as_str().unwrap()).unwrap();
        treasury.save_to(directory.path()).unwrap();
        let mut restarted = Treasury::load_from(directory.path()).unwrap();
        assert_eq!(restarted.recover_interrupted_executions().unwrap(), 1);
        assert_eq!(
            restarted.state.intents[intent["id"].as_str().unwrap()].state,
            TransactionState::Unknown
        );
        assert!(
            restarted
                .handle(
                    &token,
                    Request::ExecutePurchaseIntent {
                        intent_id: intent["id"].as_str().unwrap().to_string(),
                    },
                )
                .is_err()
        );
    }

    #[test]
    fn manual_provider_requires_fresh_non_estimated_balance_and_owner_handoff() {
        let bootstrap =
            Treasury::bootstrap("owner", Money::positive(1_000, "CAD").unwrap()).unwrap();
        let mut treasury = bootstrap.treasury;
        treasury
            .handle(
                &bootstrap.owner_token,
                Request::OwnerConfigureManualProvider {
                    credential_reference: "keychain://agent-treasury/card".to_string(),
                    provider_kind: "os-credential-store".to_string(),
                    last_four: Some("1111".to_string()),
                    balance: Money::positive(1_000, "CAD").unwrap(),
                    balance_status: BalanceStatus::OwnerConfirmed,
                    balance_ttl_secs: 60,
                },
            )
            .unwrap();
        let (_, token) = create_agent(
            &mut treasury,
            &bootstrap.owner_token,
            Policy::conservative_demo().unwrap(),
            AutonomyMode::BoundedAutonomous,
        );
        let intent = treasury
            .handle(&token, Request::CreatePurchaseIntent { request: request("manual", 500) })
            .unwrap();
        assert_eq!(intent["state"], "approval_required");
        treasury
            .handle(
                &bootstrap.owner_token,
                Request::OwnerApproveIntent {
                    intent_id: intent["id"].as_str().unwrap().to_string(),
                },
            )
            .unwrap();
        let execution = treasury
            .handle(
                &token,
                Request::ExecutePurchaseIntent {
                    intent_id: intent["id"].as_str().unwrap().to_string(),
                },
            )
            .unwrap();
        assert_eq!(execution["status"], "unknown");
        treasury
            .state
            .manual_provider
            .as_mut()
            .unwrap()
            .balance_snapshot
            .as_mut()
            .unwrap()
            .expires_at = now() - 1;
        assert!(treasury.provider_available_balance().is_err());
    }

    #[test]
    fn contradictory_decline_reconciliation_rolls_back() {
        let bootstrap =
            Treasury::bootstrap("owner", Money::positive(10_000, "CAD").unwrap()).unwrap();
        let mut treasury = bootstrap.treasury;
        let (_, token) = create_agent(
            &mut treasury,
            &bootstrap.owner_token,
            Policy::conservative_demo().unwrap(),
            AutonomyMode::BoundedAutonomous,
        );
        let mut purchase = request("charged-but-unknown", 500);
        purchase.scenario = SimulatedScenario::TimeoutAfterSubmit;
        let intent =
            treasury.handle(&token, Request::CreatePurchaseIntent { request: purchase }).unwrap();
        treasury
            .handle(
                &token,
                Request::ExecutePurchaseIntent {
                    intent_id: intent["id"].as_str().unwrap().to_string(),
                },
            )
            .unwrap();
        let before = treasury.state.clone();
        assert!(
            treasury
                .handle(
                    &bootstrap.owner_token,
                    Request::OwnerReconcile {
                        intent_id: intent["id"].as_str().unwrap().to_string(),
                        outcome: ReconciliationOutcome::Declined,
                        provider_reference: None,
                    },
                )
                .is_err()
        );
        assert_eq!(treasury.state, before);
    }

    #[test]
    fn audit_chain_detects_tampering() {
        let bootstrap =
            Treasury::bootstrap("owner", Money::positive(10_000, "CAD").unwrap()).unwrap();
        let mut treasury = bootstrap.treasury;
        treasury.state.audit[0].action = "tampered".to_string();
        assert!(treasury.verify_audit_chain().is_err());
    }

    #[test]
    fn canary_is_redacted_in_text() {
        assert!(!redact_sensitive("4111111111111111 cvv 123").contains("4111111111111111"));
        assert!(!redact_sensitive("4111-1111-1111-1111 CVC=737").contains("4111-1111"));
        assert!(!redact_sensitive("4111 1111 1111 1111 CVC=737").contains("737"));
        assert_eq!(redact_sensitive("short 123"), "short 123");
    }

    proptest! {
        #[test]
        fn money_add_sub_round_trip(a in 0_i64..1_000_000, b in 0_i64..1_000_000) {
            let left = Money::new(a, "CAD").unwrap();
            let right = Money::new(b, "CAD").unwrap();
            let sum = left.checked_add(&right).unwrap();
            prop_assert_eq!(sum.checked_sub(&right).unwrap(), left);
        }

        #[test]
        fn redaction_never_emits_long_digit_canary(digits in "[0-9]{13,19}") {
            prop_assert!(!redact_sensitive(&digits).contains(&digits));
        }
    }
}
