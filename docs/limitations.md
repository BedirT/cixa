# Limitations

- There is no authoritative KOHO balance API adapter. KOHO is manual-only unless a future officially supported API is documented and reviewed.
- Manual balances can become stale. The UI labels provider-verified, owner-confirmed, and estimated states separately; estimated or expired snapshots cannot authorize spending.
- KOHO 3-D Secure, fraud alerts, CAPTCHAs, card locks, card replacement, account recovery, and identity verification remain manual owner actions.
- The project has no outgoing Interac e-Transfer implementation.
- Merchant compatibility is not universal. Unknown forms and uncertain totals require approval or denial.
- A stored-card or browser-profile risk cannot be eliminated by the broker alone. Use ephemeral contexts and a deliberately small issuer balance.
- Issuers may decline, reverse, delay, or dispute charges independently of this ledger.
- The system is not a bank, wallet, issuer, money transmitter, payment processor, or tax/accounting system.
- The project does not claim PCI DSS compliance, formal certification, legal approval in every jurisdiction, or secure memory zeroization.
- A compromised root/administrator, kernel, hypervisor, browser runtime, dependency supply chain, dishonest owner, or compromised issuer is outside the strongest local guarantee.
- Merchant disputes, chargebacks, tax treatment, and accounting treatment require human professional advice.
- The current browser reference is a safe-denial boundary, not a claim of universal Playwright payment isolation.
- Local IPC protects against ordinary accidental exposure with OS permissions. It is not a defense against a privileged local attacker.
