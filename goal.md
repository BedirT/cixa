Build, harden, test, and fully document a publish-ready open-source project with the working name `cixa`: a local-first, provider-agnostic payment firewall and treasury control plane that lets an untrusted software agent make legitimate online purchases using a user-owned prepaid or virtual card, within a hard owner-defined autonomy envelope, without giving the agent access to account logins, raw payment credentials, security settings, or the ability to increase its own authority.

The first documented real-world setup must work with a user-owned Canadian KOHO prepaid virtual Mastercard and an Interac e-Transfer receiving address, but KOHO must be treated only as a manual card provider. The project must remain independent of KOHO, independent of any particular LLM framework, and usable by any compatible agent through MCP, a CLI, a local API, or thin SDKs.

Do not stop until the project is implemented end to end, all required documentation exists, the simulated and adversarial checkout flows work, the complete verification command passes, and the final security review has no unresolved critical or high-severity findings within the documented threat model.

## Product mission

Create a reusable open-source infrastructure component for giving software agents **bounded financial autonomy**.

The owner should be able to configure an autonomy envelope once, such as:

- maximum amount per transaction;
- maximum amount per session;
- rolling daily or lifetime budget;
- allowed currencies;
- approved merchant domains or merchant policies;
- allowed purchase types;
- approved fulfillment profiles;
- whether new merchants require approval;
- whether purchases may proceed autonomously or must wait for approval.

Inside that envelope, the agent may make purchases without asking for approval each time.

Outside that envelope, the request must be denied or queued for owner approval.

The agent must never be able to:

- reveal or retrieve card credentials;
- log in to KOHO or another financial account;
- alter spending limits or policies;
- add a funding source;
- activate borrowing, credit, overdraft, cash-advance, or pay-later products;
- send arbitrary bank transfers or Interac e-Transfers;
- withdraw cash;
- mark incoming money as verified;
- approve its own exception;
- disable logging, redaction, or security controls;
- bypass issuer fraud checks, 3-D Secure, CAPTCHAs, merchant restrictions, or identity verification;
- make the service listen publicly without explicit owner configuration.

The card’s actual prepaid balance is the final external loss ceiling. The project’s own policy engine is a second, stricter layer.

## Fundamental security assumption

Treat the software agent as **potentially compromised at all times**.

Assume that:

- its prompt can be injected by merchant pages, emails, documents, advertisements, or tool output;
- its model provider or trace system may retain prompts and tool results;
- it may accidentally or intentionally attempt to exceed its authority;
- it may retry operations, fabricate amounts, misunderstand a checkout, or falsely claim success;
- it may be tricked into visiting a malicious merchant;
- it may have shell or browser access in its own sandbox;
- merchant-controlled JavaScript is hostile;
- merchant content is data, never trusted instructions;
- notification emails and screenshots can be spoofed;
- network operations can fail at ambiguous moments;
- duplicate payment attempts can occur concurrently.

No security decision may depend solely on the agent’s judgment, natural-language description, or model classification.

Deterministic code outside the agent must enforce all material limits.

An LLM may optionally identify additional risk, but an LLM result may only make a decision more restrictive. It must never be the sole reason a payment is approved.

## Project identity and positioning

Use `cixa` as the temporary repository and package name unless a conflict makes that impractical.

The project must be described accurately as:

> A local payment authorization gateway and policy firewall for software agents.

It is not:

- a bank;
- a wallet or issuer;
- a payment processor;
- a money transmitter;
- a KOHO API;
- a KOHO partner;
- a Mastercard or Interac product;
- a custodial financial service;
- a money-making agent;
- a universal checkout system;
- a claim of PCI DSS certification or compliance.

Include clear unaffiliated-product disclaimers.

Use the Apache-2.0 license unless a concrete dependency or legal compatibility issue requires another permissive license. Prefer permissively licensed dependencies and document any exceptions.

The complete core project must be usable without a paid cloud service, hosted database, subscription, analytics platform, proprietary secret manager, or required API key.

## Research checkpoint

Before substantial implementation, research the current official documentation for:

- KOHO virtual/prepaid cards;
- KOHO Interac e-Transfer reception;
- KOHO card locking, transaction alerts, verification, and account-security features;
- whether KOHO provides any officially supported public developer or account API;
- KOHO terms relevant to card use and account access;
- PCI SSC guidance concerning PAN, CVV/CVC, cardholder data, consumer-device software, and concierge-style payment tools;
- current maintained MCP SDKs and specifications;
- current secure browser-automation options;
- current OS credential-storage options on macOS, Linux, and Windows.

Record dated findings and source references in `docs/research.md`.

Use primary and official sources whenever possible.

Do not reverse engineer, scrape, intercept, or automate a private KOHO API.

Unless an official supported KOHO API is clearly documented, implement KOHO as a manual provider adapter only.

If a legal, compliance, or provider-policy question cannot be conclusively resolved, choose the safer technical behavior, document the uncertainty, avoid compliance claims, and continue implementing the safe fallback.

## Required system model

The system has four distinct principals:

### Owner

The human cardholder and system administrator.

Only the owner may:

- provision or remove payment credentials;
- configure receive instructions;
- create, revoke, pause, or delete agents;
- define and change policies;
- approve exceptions;
- record or verify external deposits;
- reconcile provider transactions;
- operate the emergency stop;
- enable experimental autonomy modes;
- export sensitive records;
- configure secret providers.

### Agent

An untrusted software process with a narrow capability credential.

The agent may:

- inspect its effective budget;
- inspect non-sensitive capabilities;
- retrieve public receiving instructions;
- create purchase intents;
- inspect its own intents and sanitized transactions;
- execute an intent only when the policy engine has authorized autonomous execution;
- cancel an unexecuted intent;
- retrieve sanitized receipts.

### Payment broker

A separate trusted local service.

It owns:

- policy enforcement;
- budget reservation;
- idempotency;
- transaction state;
- audit logging;
- secret-provider access;
- checkout critical sections;
- redaction;
- owner approval boundaries;
- reconciliation state.

### Financial provider

The external account or card issuer where real money lives.

For the first real-world setup, this is KOHO.

The project must not pretend its internal ledger is the provider’s authoritative account balance unless a future adapter obtains that balance from an official authenticated API.

## Autonomy modes

Implement at least these modes:

### Observe

The agent can inspect budget and receiving instructions but cannot initiate spending.

### Approval required

The agent can prepare purchase intents, but every purchase requires owner approval.

This is the secure default for a newly created agent.

### Bounded autonomous

The agent may execute purchases without per-purchase approval only when every deterministic policy check passes.

The owner must explicitly enable this mode.

### Disabled or emergency stopped

All new purchase operations are denied immediately.

Pending operations must not silently resume.

The agent must never have a tool or endpoint that changes its own mode.

## Provider architecture

Define a clean provider interface so official financial APIs can be added later.

Include these initial providers:

### Simulated provider

A fully deterministic fake provider used by tests and the local demo.

It must support:

- balances;
- holds;
- approvals;
- declines;
- settlements;
- refunds;
- delayed settlement;
- ambiguous timeouts;
- incoming deposits;
- provider transaction identifiers.

### Manual prepaid-card provider

A provider for user-owned cards without an official API.

It must support:

- secure card credential references;
- owner-confirmed or imported balance snapshots;
- public receiving instructions;
- manual transaction reconciliation;
- explicit freshness metadata;
- a clear distinction between estimated, owner-confirmed, and provider-verified state.

### KOHO reference configuration

KOHO must be documentation and configuration layered on top of the manual prepaid-card provider, not hardcoded throughout the core.

The KOHO setup guide must explain how the owner can:

- create and secure their own account;
- use a dedicated virtual card;
- keep only a deliberately small amount of money exposed;
- configure a public Interac receiving address;
- keep the KOHO login identity private;
- enable strong authentication;
- configure applicable issuer-side card locks or transaction limits;
- handle KOHO fraud alerts or verification manually;
- reconcile the internal ledger with the real account;
- lock or replace the card after a risky run;
- avoid activating borrowing or credit-like features;
- understand that KOHO features and fees may change.

Do not automate KOHO login, account navigation, card replacement, card unlocking, fraud-alert approval, or account-security actions.

Do not ask for real KOHO credentials during development or testing.

## Financial model

Represent all monetary values using integer minor units and ISO 4217 currency codes.

Never use floating-point numbers for money.

Implement an append-only financial event model that can derive:

- owner-provided starting capital;
- external verified income;
- unverified incoming notifications;
- operator top-ups;
- authorized spending;
- pending holds;
- settled spending;
- failed or reversed charges;
- refunds;
- disputed or unknown transactions;
- currently reserved budget;
- remaining agent authority.

Clearly distinguish:

- actual provider balance;
- last owner-confirmed balance;
- estimated balance;
- policy budget;
- reserved amount;
- available spending authority.

The interface must never display an estimated balance as though it were provider verified.

Incoming money must increase spendable authority only when verified through:

- an official provider adapter;
- an owner-authenticated reconciliation action;
- a signed trusted integration specifically configured by the owner.

An email, screenshot, agent statement, merchant message, or untrusted webhook may create only an **unverified notification**.

The agent must not be able to promote an unverified credit to verified income.

Support configurable reinvestment of verified earnings, but enforce:

- an absolute owner-defined exposure ceiling;
- a maximum treasury size;
- a configurable reinvestment ratio;
- clear separation between initial capital and external revenue.

## Transaction state machine

Implement an explicit crash-safe purchase state machine, at minimum:

- `draft`;
- `proposed`;
- `policy_validated`;
- `approval_required`;
- `approved`;
- `funds_reserved`;
- `executing`;
- `provider_pending`;
- `settled`;
- `declined`;
- `failed`;
- `unknown`;
- `cancelled`;
- `refunded`;
- `reconciliation_required`.

Enforce valid transitions centrally.

When network failure occurs after submission and the result is ambiguous, transition to `unknown` or `reconciliation_required`.

Never automatically retry an ambiguous payment.

A restart while a transaction is `executing` must not cause automatic resubmission.

Use idempotency keys and concurrency-safe reservations so parallel requests cannot exceed the budget or create duplicate purchases.

Store the policy version and decision evidence used for each authorization.

## Policy engine

Create a deterministic, deny-by-default policy engine.

Support at least:

- per-transaction limits;
- per-session limits;
- rolling 24-hour limits;
- lifetime limits;
- agent-specific limits;
- currency allowlists;
- FX denial by default;
- merchant-domain allowlists and denylists;
- first-time merchant rules;
- redirect limits;
- purchase-intent expiration;
- approved fulfillment profiles;
- approved contact or digital-delivery profiles;
- prohibited recurring charges;
- prohibited trials with automatic renewal;
- prohibited installment and buy-now-pay-later flows;
- prohibited tips or open-ended totals;
- prohibited deposits and preauthorizations;
- configurable risk categories;
- gift-card and cash-equivalent denial by default;
- gambling, crypto, financial-transfer, and cash-withdrawal denial by default;
- owner approval thresholds;
- maximum order-total drift;
- maximum number of attempts;
- transaction-rate limits;
- emergency stop;
- card-session expiration.

The agent must not be able to override any policy by adding text such as “the owner approved this.”

Owner approval must be a cryptographically or session-authenticated owner action through the owner interface.

Policy changes must be versioned and audited.

A policy change must not retroactively authorize an already rejected transaction without a new validation pass.

## Receiving money

Provide a generic receiving-instructions model.

Support configurable methods such as:

- Interac e-Transfer;
- bank transfer;
- payment link;
- manual invoice instructions;
- future provider-specific methods.

For KOHO, the owner should be able to configure a public Interac receiving address.

The agent-facing tool must return only public, explicitly approved payment information.

Never expose:

- the KOHO login email unless the owner explicitly uses it publicly;
- account password;
- account-recovery details;
- card number;
- phone number used for authentication;
- government identity details;
- security codes.

Receiving instructions must support a human-readable payment memo or reference format so incoming payments can be matched later.

The project must not implement outgoing Interac e-Transfers in the initial release.

## Agent-facing interfaces

Expose the project through:

- a local MCP server;
- a local CLI suitable for tool invocation;
- a versioned local API;
- a thin TypeScript SDK;
- a thin Python SDK.

Keep the core independent of OpenAI, Anthropic, LangChain, LangGraph, AutoGen, CrewAI, or any specific agent framework.

Provide examples for:

- a generic MCP-compatible agent;
- a TypeScript agent;
- a Python agent.

Agent-facing operations should include equivalents of:

- `cixa_get_status`;
- `cixa_get_capabilities`;
- `cixa_get_budget`;
- `cixa_get_receive_instructions`;
- `cixa_create_purchase_intent`;
- `cixa_get_purchase_intent`;
- `cixa_execute_purchase_intent`;
- `cixa_cancel_purchase_intent`;
- `cixa_list_transactions`;
- `cixa_get_receipt`.

Use strict schemas with bounded input sizes and `additionalProperties: false` where applicable.

Do not provide agent-facing operations for:

- viewing credentials;
- changing policies;
- changing limits;
- adding cards;
- approving exceptions;
- recording deposits;
- reconciling transactions;
- exporting sensitive data;
- changing server configuration;
- disabling safeguards.

Agent capability credentials must be:

- revocable;
- scoped;
- expiring or session-bound;
- associated with a specific agent and policy;
- stored hashed where feasible;
- separate from owner credentials.

Prefer local stdio, Unix-domain sockets, or Windows named pipes.

Do not listen on a public network interface by default.

Any optional TCP mode must require explicit configuration, strong authenticated encryption, and prominent warnings.

## Owner interfaces

Provide:

- a secure owner CLI;
- a simple local owner dashboard;
- emergency stop;
- agent creation and revocation;
- policy creation and editing;
- pending approval review;
- transaction reconciliation;
- provider and card provisioning;
- receive-instruction configuration;
- session arming;
- audit-log inspection;
- sanitized export.

The owner dashboard must:

- bind to loopback only by default;
- use proper CSRF protection;
- use secure, HTTP-only, same-site cookies where applicable;
- reject untrusted origins;
- avoid third-party scripts, fonts, analytics, and CDNs;
- display whether data is provider verified, owner confirmed, or estimated;
- make the emergency stop visually prominent;
- make experimental or unsafe modes visually unmistakable;
- never display a full PAN or CVV after initial entry.

Do not place owner operations in the MCP server used by the agent.

## Secret handling

Design secrets as a separate security subsystem.

Protected assets include:

- PAN;
- expiry;
- CVV/CVC;
- billing identity;
- shipping identity;
- phone number;
- account aliases;
- owner authentication material;
- agent capability credentials;
- audit-log integrity keys.

Requirements:

- never put secrets in command-line arguments;
- never put secrets in source code;
- never put secrets in test fixtures;
- never put secrets in environment variables when a safer file-descriptor, pipe, or OS credential mechanism is available;
- never include secrets in logs, traces, screenshots, browser recordings, crash reports, telemetry, exceptions, MCP output, API errors, or analytics;
- show only masked values such as last four digits;
- use OS credential facilities where practical;
- use strong authenticated encryption for any encrypted local fallback;
- separate encryption keys from encrypted data;
- restrict files and sockets with OS permissions;
- disable core dumps where practical;
- zeroize sensitive memory on a best-effort basis;
- document where language or browser-runtime behavior prevents guaranteed zeroization;
- use synthetic test credentials only.

Treat CVV handling as a compliance-sensitive architectural issue.

Before implementing CVV persistence or retrieval, review current PCI SSC guidance and document the result.

The secure default must not persist CVV in the database, config files, logs, browser profiles, or ordinary application storage.

Implement a pluggable just-in-time `SecretProvider` abstraction with at least:

- interactive owner entry;
- volatile session secret;
- an owner-controlled local secret-helper protocol;
- a simulated test provider.

An optional owner-controlled local credential-store adapter may be implemented only with:

- explicit opt-in;
- strong warnings;
- no claim of PCI compliance;
- isolation from the agent;
- no ability for the agent to invoke arbitrary secret lookups;
- strict binding to a card reference and approved payment operation.

After a payment attempt, clear transaction-specific secret material as soon as practical.

The project must never claim that encrypted CVV storage is automatically compliant.

## Fulfillment and personal information

Create owner-managed fulfillment profiles such as:

- `home`;
- `office`;
- `digital-email`;
- `pickup-location`.

The agent should see profile labels and only the minimum metadata needed to choose one.

The broker should fill the actual address, contact information, and billing information during the secure checkout section.

By default, the agent must not be able to ship an item to an arbitrary newly supplied address.

An owner policy may permit additional destinations, but the agent cannot change that policy itself.

Redact personal information from receipts returned to the agent unless explicitly needed.

## Secure checkout architecture

Create an executor interface with at least:

### Simulated checkout executor

Used for deterministic tests.

### Secure handoff executor

The agent prepares a purchase and checkout state, then hands control to the trusted broker.

The broker:

1. obtains exclusive control of the checkout context;
2. suspends all agent browser access;
3. verifies merchant origin, redirect chain, amount, currency, items, fulfillment, and recurring-payment indicators;
4. performs policy validation again immediately before submission;
5. reserves funds transactionally;
6. obtains payment secrets just in time;
7. fills payment and owner-profile fields;
8. submits exactly once;
9. observes the result without exposing secrets;
10. clears sensitive state;
11. destroys or sanitizes the browser context;
12. returns only a sanitized result and receipt.

### Experimental brokered-browser executor

Provide a reference Playwright or equivalent implementation that allows an agent to navigate and prepare a cart only through a controlled browser gateway.

During the payment critical section:

- revoke or pause agent control;
- prevent agent JavaScript execution;
- prevent screenshots, traces, videos, DOM snapshots, console logs, and network-body recording from capturing secrets;
- prevent the agent from reading autofilled fields;
- prevent clipboard and local-file access;
- use an ephemeral browser profile;
- destroy the profile after the purchase;
- never expose a remote debugging endpoint to the agent;
- do not reuse payment cookies or card data across unrelated sessions unless explicitly designed and documented.

The agent must never receive direct CDP, WebDriver, or Playwright access to the payment-critical browser process.

## Merchant and checkout validation

Autonomous checkout must not blindly fill card data into arbitrary merchant-controlled text fields.

Implement a tiered trust model:

### Trusted hosted-payment fields

Prefer recognized cross-origin hosted payment fields or hosted checkout pages where card data is sent directly to a known payment processor rather than exposed to merchant JavaScript.

### Explicitly owner-approved merchant integration

Permit merchant-specific adapters or domains approved by the owner.

### Unknown or merchant-controlled payment form

Require owner approval or deny in bounded-autonomous mode.

Do not claim universal merchant support.

If the broker cannot confidently determine:

- final amount;
- currency;
- merchant origin;
- whether the purchase is recurring;
- whether the card will be saved;
- whether a trial auto-renews;
- whether an additional amount may be added later;
- whether the destination is approved;

then it must not autonomously submit the payment.

Validate every URL and redirect:

- HTTPS only by default;
- canonicalize hostnames;
- handle internationalized domain names safely;
- reject embedded credentials;
- reject malformed ports;
- reject `file:`, `data:`, `javascript:`, browser-internal, and extension URLs;
- block localhost;
- block loopback, private, link-local, multicast, and cloud-metadata addresses;
- revalidate DNS and IP ranges after redirects;
- mitigate DNS rebinding and SSRF;
- set redirect-count and navigation-time limits.

Verify the final visible total against the intent’s maximum total immediately before submission.

Do not trust hidden fields or agent-provided totals alone.

## Retry and ambiguity safety

Payments must be exactly-once from the broker’s perspective wherever possible.

Requirements:

- caller-supplied or broker-generated idempotency key;
- one active execution per intent;
- transactional budget reservation;
- no blind browser refresh after submit;
- no automatic form resubmission;
- no retry after timeout without reconciliation;
- clear `unknown` state for ambiguous outcomes;
- owner-visible reconciliation workflow;
- provider reference captured when available;
- receipts hashed and associated with the intent;
- duplicate-intent detection.

## Audit logging

Create a structured, append-only audit log.

Record:

- actor;
- agent identity;
- action;
- timestamp;
- intent ID;
- policy version;
- decision;
- decision reasons;
- merchant origin;
- amount and currency;
- state transition;
- owner approval where applicable;
- relevant provider reference;
- reconciliation event.

Never log secrets.

Make the log tamper-evident using a hash chain or keyed integrity mechanism.

Store integrity material separately from ordinary log data.

Document that a compromised OS administrator remains outside the strongest local guarantees unless an external log sink is configured.

Provide sanitized audit export.

## Threat model

Create `THREAT_MODEL.md` before finishing the implementation.

Use a structured methodology such as STRIDE plus explicit abuse cases.

Cover at minimum:

- prompt injection;
- malicious merchant pages;
- compromised agent process;
- malicious browser content;
- secret exfiltration;
- model-provider logging;
- screenshots and browser traces;
- local unprivileged process attacks;
- socket and API authentication;
- token theft;
- replay;
- CSRF;
- XSS;
- SSRF;
- DNS rebinding;
- Unicode and homograph domains;
- redirect manipulation;
- amount substitution;
- currency substitution;
- hidden subscriptions;
- preauthorization holds;
- stored-card consent;
- duplicate submission;
- network loss after submit;
- process crash;
- concurrent double-spend;
- ledger tampering;
- forged income;
- spoofed email notifications;
- compromised dependencies;
- malicious package-install scripts;
- unsafe updates;
- owner-interface takeover;
- accidental public exposure.

State assumptions and out-of-scope risks clearly, including:

- compromised kernel or root/administrator;
- compromised card issuer;
- dishonest owner;
- merchant disputes and chargebacks;
- tax and accounting advice;
- universal merchant compatibility;
- legal approval for every jurisdiction;
- formal PCI certification.

For every in-scope threat, identify preventive, detective, and recovery controls.

## Recommended implementation structure

Prefer a small security-focused Rust core and daemon for:

- policy enforcement;
- money arithmetic;
- transaction state;
- ledger;
- audit log;
- credential references;
- owner and agent authorization;
- local IPC;
- reconciliation.

Use TypeScript where it materially improves:

- MCP integration;
- browser automation;
- local dashboard;
- TypeScript SDK.

Provide a Python SDK as a thin generated or hand-written client.

A suggested structure is:

```text
/
├── apps/
│   ├── daemon/
│   ├── owner-dashboard/
│   ├── mcp-server/
│   └── test-merchant/
├── crates/
│   ├── domain/
│   ├── policy/
│   ├── ledger/
│   ├── audit/
│   ├── vault/
│   ├── provider/
│   └── checkout/
├── packages/
│   ├── sdk-typescript/
│   ├── sdk-python/
│   └── browser-executor/
├── examples/
├── docs/
├── scripts/
├── tests/
├── AGENTS.md
├── PLAN.md
├── PROGRESS.md
├── THREAT_MODEL.md
├── SECURITY.md
├── CONTRIBUTING.md
├── LICENSE
└── README.md
```

This structure is guidance, not an absolute requirement.

If a materially simpler or safer design is chosen, document the decision in an ADR before implementing it.

Use current stable, maintained dependencies and commit lockfiles.

Avoid unnecessary dependencies.

Avoid unsafe Rust unless narrowly justified and documented.

Do not use generic `eval`, dynamically execute agent-provided code, or shell out with untrusted strings.

## Local isolation

Design deployment so the payment broker can run separately from the agent.

Provide documented configurations for:

- macOS `launchd` or a dedicated local process;
- Linux `systemd` or a dedicated service account;
- Windows service or named-pipe operation;
- agent running in a container while the broker runs on the host.

The agent should have access only to the policy-bound interface, not:

- the daemon data directory;
- the secret store;
- the owner UI session;
- browser debugging ports;
- raw audit files;
- provider credentials.

Bind local files, sockets, and processes using least privilege.

## Testing requirements

Build a comprehensive test suite.

### Unit tests

Cover:

- money arithmetic;
- overflow and underflow;
- currency mismatches;
- policy evaluation;
- state transitions;
- token scopes;
- redaction;
- domain canonicalization;
- URL validation;
- budget reservation;
- refund handling;
- incoming-fund verification;
- audit-chain validation.

### Property-based tests

Prove important invariants, including:

- available authority never becomes negative;
- concurrent reservations never exceed the limit;
- an agent cannot increase its own budget;
- duplicate idempotency keys cannot settle twice;
- invalid state transitions are rejected;
- floats are never introduced into financial calculations;
- unverified income never becomes spendable;
- redaction never emits the full synthetic PAN or CVV.

### Integration tests

Cover:

- daemon plus simulated provider;
- owner creation;
- agent issuance and revocation;
- policy updates;
- emergency stop;
- MCP calls;
- TypeScript SDK;
- Python SDK;
- audit export;
- restart recovery;
- manual reconciliation.

### Adversarial checkout test application

Build a local fake merchant and malicious checkout laboratory that can simulate:

- normal one-time purchase;
- amount increase before submit;
- currency switch;
- hidden recurring checkbox;
- free trial with auto-renewal;
- card-saving option;
- tip field;
- preauthorization;
- delayed settlement;
- decline;
- timeout before submit;
- timeout after submit;
- duplicate form submission;
- misleading success page;
- cross-origin hosted fields;
- merchant-controlled card fields;
- redirect to another domain;
- redirect to localhost;
- DNS-rebinding-like behavior where feasible;
- prompt injection in product text;
- malicious JavaScript attempting to inspect payment fields;
- screenshot and trace leakage attempts;
- forged deposit notification;
- spoofed receipt;
- browser crash during execution.

The secure broker must deny or safely handle each scenario.

### Secret-canary tests

Use a synthetic test PAN and CVV as canaries.

After each test run, automatically inspect:

- application logs;
- browser traces;
- screenshots;
- videos;
- SQLite files;
- crash artifacts;
- MCP output;
- API responses;
- generated reports;
- CI artifacts.

Fail the test if a full canary secret appears anywhere it should not.

### Security and supply-chain checks

Include:

- formatting;
- linting;
- type checking;
- unit tests;
- integration tests;
- end-to-end tests;
- dependency vulnerability scanning;
- license checking;
- secret scanning;
- static analysis;
- Rust dependency audit;
- JavaScript dependency audit;
- CodeQL-compatible GitHub workflow;
- SBOM generation;
- fuzz targets for parsers and IPC;
- documented update process.

Security-critical policy and ledger modules should have high branch coverage, with a target of at least 90% unless a specific exception is documented.

## Demo requirements

Create a completely local demo requiring no real money and no external account.

The demo must prove this flow:

1. Start the daemon, dashboard, MCP server, simulated provider, and test merchant.
2. Create an owner.
3. Create a simulated card with a synthetic balance.
4. Create an agent with a strict policy and bounded-autonomous mode.
5. Retrieve the agent’s budget.
6. Retrieve public receiving instructions.
7. Create a valid low-value purchase intent.
8. Execute it autonomously.
9. Show the resulting reservation, settlement, sanitized receipt, and remaining budget.
10. Retry with the same idempotency key and prove that no duplicate charge occurs.
11. Attempt an over-budget purchase and prove it is denied.
12. Attempt a recurring purchase and prove it is denied.
13. Attempt a currency substitution and prove it is denied.
14. Attempt a malicious merchant-controlled card form and prove it requires approval or is denied.
15. Trigger the emergency stop and prove all new spending is denied.
16. Run the secret-canary scan and prove no sensitive value leaked.

Provide one command for the demo and one canonical verification command.

Examples:

```bash
./scripts/demo
./scripts/verify
```

Create PowerShell equivalents where practical.

## Documentation deliverables

Create at least:

### `README.md`

Include:

- what the project does;
- what it does not do;
- architecture overview;
- quickstart in simulated mode;
- owner and agent concepts;
- autonomy modes;
- MCP example;
- security warnings;
- project status;
- limitations;
- unaffiliated-product disclaimer.

### `PLAN.md`

Maintain the implementation plan and checkpoints.

### `PROGRESS.md`

Keep a concise running record of:

- current checkpoint;
- completed work;
- tests run;
- security findings;
- remaining work;
- blockers.

### `THREAT_MODEL.md`

Complete threat model with trust boundaries and mitigations.

### `SECURITY.md`

Include:

- supported versions;
- responsible disclosure process;
- GitHub private vulnerability reporting instructions;
- secret-handling policy;
- what must never be included in a vulnerability report;
- incident-response basics.

### `docs/architecture.md`

Include Mermaid diagrams for:

- trust boundaries;
- process architecture;
- purchase state machine;
- secure checkout critical section;
- provider abstraction;
- owner versus agent interfaces.

### `docs/koho-setup.md`

Provide a careful, dated, manual setup guide based on current official sources.

Do not include real credentials.

Do not claim official KOHO support or affiliation.

### `docs/security-model.md`

Explain why the agent is treated as untrusted and why prompt instructions alone are not security controls.

### `docs/credential-handling.md`

Explain:

- PAN handling;
- CVV limitations;
- secret-provider modes;
- OS keychain behavior;
- browser-process exposure;
- redaction;
- remaining risks;
- absence of formal PCI certification.

### `docs/agent-integration.md`

Document MCP, CLI, API, TypeScript, and Python usage.

### `docs/provider-adapters.md`

Document the provider interface and how future official providers can be added.

### `docs/checkout-adapters.md`

Document secure handoff, hosted fields, merchant-specific adapters, and failure behavior.

### `docs/limitations.md`

Be candid about:

- no authoritative KOHO balance API unless officially available;
- manual reconciliation;
- 3-D Secure and fraud-alert intervention;
- non-universal merchant compatibility;
- persistent-card risk;
- issuer and merchant behavior;
- compliance uncertainty;
- local-host compromise;
- browser-runtime limitations.

### `docs/incident-response.md`

Explain how to:

- emergency stop;
- revoke an agent;
- stop the daemon;
- lock the card manually;
- rotate credentials;
- inspect the audit log;
- reconcile ambiguous transactions;
- check for secret leakage.

### ADRs

Record major decisions, especially:

- language and process boundaries;
- secret storage;
- CVV strategy;
- local IPC;
- browser executor;
- ledger model;
- hosted-payment-field trust model.

## Development process

Work in checkpoints.

### Checkpoint 1: Research and architecture

Create:

- `PLAN.md`;
- `PROGRESS.md`;
- `docs/research.md`;
- initial ADRs;
- `THREAT_MODEL.md`;
- API and data-model proposal.

Do not begin real provider automation.

### Checkpoint 2: Core domain and simulator

Implement:

- money types;
- state machine;
- policy engine;
- provider interface;
- simulated provider;
- append-only ledger;
- audit chain;
- core unit and property tests.

### Checkpoint 3: Daemon and authorization

Implement:

- local daemon;
- owner/agent identity separation;
- scoped capability credentials;
- local IPC;
- owner CLI;
- emergency stop;
- restart recovery.

### Checkpoint 4: Agent interfaces

Implement:

- MCP server;
- versioned API;
- CLI tools;
- TypeScript SDK;
- Python SDK;
- examples.

### Checkpoint 5: Owner dashboard and secret subsystem

Implement:

- local dashboard;
- provider configuration;
- policy editor;
- agent management;
- approvals;
- reconciliation;
- secret providers;
- secure session arming.

### Checkpoint 6: Checkout execution

Implement:

- simulated executor;
- secure handoff;
- experimental brokered browser;
- payment critical section;
- safe denial behavior;
- test merchant.

### Checkpoint 7: Adversarial validation

Implement and run:

- malicious checkout cases;
- prompt-injection cases;
- concurrency tests;
- crash recovery;
- secret-canary scans;
- fuzzing;
- static analysis;
- dependency scans.

### Checkpoint 8: Documentation and release readiness

Complete:

- all documentation;
- demo;
- CI;
- SBOM;
- packaging;
- versioning;
- security review;
- known limitations;
- publish-ready repository state.

Make logical local commits after meaningful checkpoints when Git is available.

Do not push, publish packages, create cloud resources, make the repository public, or perform any external irreversible action without explicit owner approval.

## Decision-making rules

Do not ask routine implementation questions.

Make defensible choices, record them in ADRs, and continue.

Pause only when:

- real financial credentials would be required;
- a real transaction would be submitted;
- an external account must be created;
- code or packages would be publicly published;
- an irreversible external action is required;
- two requirements are genuinely impossible to reconcile safely.

When a feature cannot be implemented safely, do not silently weaken the security model.

Instead:

1. document the limitation;
2. implement the safest fallback;
3. add an explicit unsupported or approval-required result;
4. continue completing the rest of the project.

## Hard prohibitions

Do not:

- use a real card in tests;
- request the owner’s KOHO password;
- automate KOHO authentication;
- store login credentials;
- reverse engineer private financial APIs;
- bypass fraud controls;
- bypass 3-D Secure;
- bypass CAPTCHA or anti-bot controls;
- submit a real purchase;
- persist secrets in plaintext;
- expose secrets to the agent;
- include secrets in screenshots or traces;
- trust email as authoritative settlement evidence;
- retry an ambiguous payment automatically;
- use floating-point money;
- let the agent change its own policy;
- let the agent access owner endpoints;
- bind publicly by default;
- require a paid service;
- add telemetry enabled by default;
- claim PCI compliance;
- claim universal merchant compatibility;
- claim formal security certification;
- leave placeholder implementations in security-critical paths;
- mark the project complete while known critical or high findings remain unresolved.

## Canonical verification

Create a single root command:

```bash
./scripts/verify
```

It must run all required:

- format checks;
- lint checks;
- type checks;
- Rust tests;
- TypeScript tests;
- Python tests;
- property tests;
- integration tests;
- end-to-end tests;
- adversarial checkout tests;
- secret-canary scans;
- dependency audits;
- static analysis;
- license checks;
- SBOM generation;
- documentation validation.

CI must run the same canonical verification path rather than a weaker parallel path.

## Definition of done

The goal is complete only when all of the following are true:

- the repository builds from a clean checkout;
- simulated mode requires no paid services or external accounts;
- the local demo completes successfully;
- the owner and agent interfaces are strictly separated;
- the agent cannot obtain payment credentials;
- the agent cannot change policies or increase limits;
- bounded-autonomous spending works in the simulated safe checkout;
- over-budget, recurring, currency-changing, duplicate, malicious, and ambiguous purchases are safely denied or quarantined;
- crash recovery does not duplicate a payment;
- incoming funds cannot be forged by the agent;
- the KOHO manual-provider guide is complete and candid;
- no KOHO account automation or private API reverse engineering exists;
- secret-canary tests pass;
- no full synthetic PAN or CVV appears in logs or artifacts;
- all automated checks pass through `./scripts/verify`;
- the threat model is complete;
- the security review is complete;
- there are no unresolved critical or high-severity findings within the stated threat model;
- all remaining risks are documented clearly;
- the repository is ready for human review and eventual open-source publication;
- no real financial transaction has been made.

At completion, provide a concise final report containing:

- architecture summary;
- security-boundary summary;
- commands to run the demo and verification;
- test and scan results;
- important residual risks;
- exact files requiring human review before public release;
- the manual steps needed for a later KOHO test without asking for or exposing real credentials.