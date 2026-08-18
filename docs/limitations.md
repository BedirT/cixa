# Limitations

- There is no authoritative KOHO balance or transaction API adapter. Balance entry and final reconciliation are owner-manual unless a future officially supported API is documented and reviewed.
- Manual balances can become stale. The UI labels provider-verified, owner-confirmed, and estimated states separately; estimated or expired snapshots cannot authorize spending.
- KOHO 3-D Secure, fraud alerts, CAPTCHAs, card locks, card replacement, account recovery, and identity verification remain manual owner actions.
- The project has no outgoing Interac e-Transfer implementation.
- Merchant compatibility is not universal. Controlled checkout requires an owner-maintained profile, complete visible evidence, an exact redirect chain, and cross-origin hosted payment fields. CAPTCHAs, 3-D Secure interaction, owner login, and merchant-controlled card fields are unsupported.
- A stored-card or browser-profile risk cannot be eliminated by the broker alone. Use ephemeral contexts and a deliberately small issuer balance.
- Issuers may decline, reverse, delay, or dispute charges independently of this ledger.
- The system is not a bank, wallet, issuer, money transmitter, payment processor, or tax/accounting system.
- The project does not claim PCI DSS compliance, formal certification, legal approval in every jurisdiction, or secure memory zeroization.
- While a payment session is armed, the helper process holds the card in volatile memory. It does not persist it, but JavaScript, process memory, swap, crash reporting, and a compromised host remain residual risks.
- A compromised root/administrator, kernel, hypervisor, browser runtime, dependency supply chain, dishonest owner, or compromised issuer is outside the strongest local guarantee.
- The broker owner UID and owner-reviewed checkout executables are trusted. File permission and path checks reject accidental or cross-identity substitution, but do not defend against same-UID replacement races or a deliberately daemonizing owner executable.
- Merchant disputes, chargebacks, tax treatment, and accounting treatment require human professional advice.
- The current browser reference is a safe-denial boundary, not a claim of universal Playwright payment isolation.
- Browser submission cannot authenticate issuer settlement. Every real controlled submission requires owner reconciliation and must never be automatically retried after timeout or ambiguity.
- Codex, Claude Code, or another unrestricted agent running under the owner's OS identity is not isolated by file permissions. Use a separate identity or container; the skill is behavioral guidance, not a sandbox.
- Docker reduces setup mistakes but does not protect an owner who mounts `cixa-owner-data`, the Docker socket, host credentials, or a privileged host filesystem into the agent. The supplied Compose boundary is part of the security model.
- Local IPC protects against ordinary accidental exposure with OS permissions. It is not a defense against a privileged local attacker.
