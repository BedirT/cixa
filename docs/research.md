# Dated Research Record

Research checked on 2026-08-16 using official or primary sources where available. Product behavior, terms, SDK versions, and security guidance can change. Recheck every source before a real deployment.

## KOHO

- [What is KOHO and how does it work?](https://help.koho.ca/en_us/what-is-koho-and-how-does-it-work-rJETQ3DEGl) was updated July 17, 2026. The help article describes KOHO's physical and virtual prepaid Mastercard and spending from a loaded balance.
- [Virtual versus physical card](https://help.koho.ca/en_us/whats-the-difference-between-a-virtual-card-and-a-physical-card-B1DFS2vEGv) describes the virtual card as available in the app and linked to the KOHO balance. This project therefore treats the virtual card as a manually provisioned owner asset, not an API resource.
- [Third-party e-Transfer](https://help.koho.ca/en_us/what%27s-a-3rd-party-e-transfer-Bycs9jcUWe) says a third party e-Transfer can be sent to the unique email address shown in-app. The receiving address is public only when the owner deliberately configures it here.
- [Sending an e-Transfer](https://help.koho.ca/en_us/how-do-i-send-an-e-transfer-from-my-koho-account-to-another-account-HkEnciq8l) documents sending behavior and account limits. It is not an authorization for this project to send money. Outgoing Interac e-Transfers are not implemented.
- [Transaction alerts](https://help.koho.ca/en_us/transaction-alerts%3A-what-you-need-to-know-r1Raco5Ll) says alerts are handled in the KOHO mobile app and declined fraud-suspect purchases may require in-app verification. The broker never automates that verification.
- [Fraudulent transaction response](https://help.koho.ca/en_us/what-do-i-do-when-i-spot-a-fraudulent-transaction-on-my-account-rkHTqo98g) instructs a user to lock the card in the KOHO app and discusses pending versus settled disputes. The incident runbook points the owner to that manual control.
- [Two-step verification](https://help.koho.ca/en_us/set-up-two-step-verification-HkZqqo9IZx) describes password plus a verification code and recommends an authenticator app as the more secure option. The owner, not the broker or agent, enables it.
- [KOHO legal terms](https://www.koho.ca/legal/) state that the prepaid card balance is used for card transactions, restrictions and limits may apply, and terms can change. The guide makes no claim that a particular plan, fee, limit, or product feature is permanent.

An official supported public developer/account API was not identified during the official-help and legal-page review on this date. That is not evidence that no private or future API exists. The safe design choice is a manual adapter and no scraping, reverse engineering, interception, login automation, or private endpoint use.

## PCI SSC and Card Data

- [PCI SSC FAQ 1319](https://www.pcisecuritystandards.org/faqs/1319/) states that card verification codes are sensitive authentication data and must not be stored after authorization, even encrypted.
- [PCI Software Security Framework FAQ](https://www.pcisecuritystandards.org/documents/FAQs-for-PCI-Software-Security-Framework-v2.pdf) describes eligibility boundaries and is not a certification for this project.

The project does not claim PCI DSS compliance. CVV is not persisted. The secret-provider boundary is intentionally conservative and documents remaining browser and local-runtime exposure.

## MCP and Browser Boundaries

- [MCP TypeScript SDK v2](https://ts.sdk.modelcontextprotocol.io/v2/) and [the first-server guide](https://ts.sdk.modelcontextprotocol.io/v2/get-started/first-server) document `@modelcontextprotocol/server`, Zod schemas, stdio transport, and the rule that stdout is the protocol channel. The adapter logs only to stderr and uses strict schemas.
- [MCP specification](https://modelcontextprotocol.io/specification/2025-03-26/index) is the protocol source of truth. The project uses local stdio for the MCP boundary and a separate local Unix socket for the broker.
- [Playwright browser contexts](https://playwright.dev/docs/browser-contexts) and [trace viewer](https://playwright.dev/docs/trace-viewer) document ephemeral contexts and trace artifacts. The shipped experimental adapter uses a fresh owner-only process and context, leaves capture channels disabled, accepts only reviewed cross-origin hosted-field integrations, and destroys the context after one attempt. A future universal adapter would need broader merchant-specific proof; unsupported configurations continue to fail closed rather than exposing a payment browser to an agent.

## OS Credential Facilities

- [Apple Keychain](https://developer.apple.com/documentation/Security/storing-keys-in-the-keychain?changes=_5_8) describes Keychain Services as the appropriate place for small secrets such as passwords and cryptographic keys.
- [freedesktop Secret Service](https://specifications.freedesktop.org/secret-service/latest/) defines a desktop secret collection and session protocol for Linux environments.
- Windows deployments should use the platform Credential Manager or a documented owner-controlled helper; the adapter is not silently implemented because a cross-platform secret-store abstraction must preserve agent isolation and explicit user authentication.

The current code provides interactive, volatile, owner-helper, and simulated provider categories as a narrow abstraction. Platform-specific storage remains explicit opt-in work, not an implicit claim of secure keychain integration.

## Research Decisions

1. Manual KOHO adapter only, with explicit freshness and verification labels.
2. CVV never persisted and no real credential collection in tests.
3. Local stdio/Unix socket by default; no public network listener.
4. MCP tools are agent-only and schemas reject unknown properties.
5. Browser automation is a safe-denial boundary until a separately reviewed executor can prove critical-section isolation.
