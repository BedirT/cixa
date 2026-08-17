# Cixa Owner Console Implementation Plan

## Goal

Replace the raw JSON dashboard with a complete, local-only owner console that makes Cixa's actual security model understandable and operable. The console must use the real broker API, preserve owner and agent isolation, never collect payment credentials, and expose ambiguous outcomes without offering unsafe retries.

## Design Contract

The console follows the delivered `cixa_design.html` direction: an airy blue-gray background, soft ambient color, translucent white panels, blue-gray ink, restrained green, amber, and clay status colors, an editorial serif display face, and accessible sans-serif controls. The supplied Cixa SVG mark is the only brand asset. No external fonts, scripts, analytics, CDN assets, or cloud services are allowed.

The implementation intentionally differs from the design brief in these places:

- A local unlock form exchanges the separate dashboard token once for a random, per-process session. This avoids leaving a reusable Basic credential in the browser authentication cache, where a local process taking over the port could collect it from polling.
- There is no fake provider-health integration or live bank balance. The UI labels simulated, owner-confirmed, estimated, and stale evidence exactly as the broker reports them.
- Mobile supports monitoring, request decisions, emergency stop, and reconciliation. Dense policy editing remains usable but is optimized for desktop and tablet.
- The hostile checkout laboratory remains a test fixture and is not presented as product UI.

## Information Architecture

The authenticated single-page console has four routes implemented with hash navigation so it works from the standard-library HTTP bridge:

1. **Today**: fortress status, authoritative broker budget usage, provider evidence, pending decisions, reconciliation, and recent activity.
2. **Ledger**: a filterable intent history with current intent and receipt details, bound checkout facts, one-time decisions, manual handoff, and guided reconciliation. No retry action exists.
3. **Agents**: agent health, modes, expiration, budget use, merchant trust, capability lifecycle, session arming, and a form-based policy editor.
4. **Trust**: local security boundaries plus provider references, receiving instructions, deposit verification, human-readable audit events, technical hashes, and sanitized export.

Emergency stop is global and persistent. Activating it is a deliberate confirmation. Resuming is a separate confirmation and explicitly states that invalidated requests do not restart.

## Broker And Bridge Work

The current owner dashboard payload already exposes agents, policies, transactions, approvals, reconciliation items, provider evidence, receiving instructions, audit count, and unsafe-mode labels. Extend it narrowly where real UI actions require missing behavior:

- Add an owner-only `owner_deny_intent` operation. It may cancel only an unexecuted `approval_required` intent and must create an owner audit entry.
- Expose owner begin/complete manual handoff routes through the bridge without accepting payment material.
- Add safe intent and receipt GET routes for detail views.
- Serve the local SVG logo with the same authentication, CSP, and no-store headers as other dashboard assets.
- Preserve exact body schemas, CSRF, Origin, Host, session-cookie, response-size, and loopback controls.

## Frontend Work

Use dependency-free HTML, CSS, and JavaScript to keep the local dashboard auditable and compatible with the existing bridge.

- Build semantic application chrome with desktop sidebar, compact mobile navigation, skip link, live connection state, refresh state, theme control, and global emergency status.
- Render all broker data into human-readable cards, tables, timelines, badges, forms, dialogs, and empty states. Raw JSON is available only inside optional technical evidence disclosures.
- Use one central state store and one request wrapper. Every mutation shows pending, success, and rejected states and refreshes authoritative broker data.
- Format money from integer minor units with an explicit currency. Format timestamps locally while retaining machine-readable values.
- Never use `innerHTML` with broker-controlled data. Build dynamic content with DOM nodes and `textContent`.
- Keep one-time purchase approval separate from durable merchant trust.
- Make authority-increasing policy changes explicit in the confirmation summary.
- Make ambiguous transaction states visually and semantically distinct from ordinary failures.

## Required States

Implement and verify:

- Loading, empty, connected, broker-offline, stale-data, mutation-pending, success, validation error, and broker-rejected states.
- Active, approval-required, allowed, denied, executing, settled, declined, cancelled, refunded, unknown, provider-pending, and reconciliation-required intent states.
- Observe, approval-required, bounded-autonomous, disabled, expired, and revoked agent states.
- Simulated, manual, owner-confirmed, estimated, stale, and unsafe provider evidence.
- Desktop at 1440px, tablet at 834px, and mobile at 390px.
- Light and dark themes, reduced motion, 200 percent zoom, keyboard navigation, visible focus, and non-color status cues.

## Verification

Completion requires evidence from all of the following:

1. Rust unit tests for owner denial authorization, state transitions, persistence, and audit behavior.
2. Dashboard bridge integration tests for every GET and mutation route, authentication, exact schemas, CSRF, and safe errors.
3. Browser workflow tests against a real local Cixa daemon and dashboard for navigation, request approval and denial, agent mode changes, policy editing, reconciliation, audit expansion/export, theme persistence, emergency stop, and responsive layouts.
4. Accessibility checks for landmarks, labels, dialog focus, keyboard operation, live regions, reduced motion, and absence of horizontal overflow at target widths.
5. Visual screenshots of the populated overview, request detail, agent policy, reconciliation flow, and mobile console, inspected before README use.
6. The canonical `./scripts/verify` gate on the exact final commit.
7. Independent product/code review and independent adversarial security review. Both must report `NO ISSUES`; any finding restarts the affected verification and review loop.

## Commit Sequence

1. Broker and bridge capabilities with focused tests.
2. Owner-console structure, navigation, rendering, and accessible workflows.
3. Browser verification, visual evidence, and README update.
4. Reviewer-driven corrections, with each correction committed separately when it changes behavior or evidence.
