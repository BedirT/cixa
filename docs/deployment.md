# Local Deployment

Docker Compose is the primary supported deployment. Start with [Docker deployment](docker.md) and the repository's `compose.yaml`. It supplies separate owner and agent identities, a private owner volume, a narrow agent IPC volume, a packaged checkout browser, loopback-only owner UI, health checks, and a network-disabled MCP bridge.

Native deployment is an advanced alternative. The broker and the agent must still run as separate OS identities. The agent receives only the scoped interface and token file. It must not mount the broker data directory, secret-helper socket, owner dashboard session, raw audit files, or browser debugging ports.

## macOS launchd

Create an owner-reviewed `~/Library/LaunchAgents/com.example.cixa.plist` with absolute paths and a private data directory. The important arguments are:

```xml
<key>ProgramArguments</key>
<array>
  <string>/absolute/path/target/release/cixa</string>
  <string>serve</string>
  <string>--data-dir</string>
  <string>/Users/OWNER/.local/cixa</string>
</array>
<key>RunAtLoad</key><true/>
<key>Umask</key><integer>63</integer>
<key>StandardOutPath</key><string>/Users/OWNER/.local/cixa/daemon.out</string>
<key>StandardErrorPath</key><string>/Users/OWNER/.local/cixa/daemon.err</string>
```

Review the plist before `launchctl bootstrap gui/$UID ...`. Do not put a token or card secret in the plist.

This per-user LaunchAgent is for simulated development only. Manual-provider mode requires a broker daemon and agent process under distinct macOS UIDs, a dedicated shared IPC group, agent token and socket paths outside the broker's private directory, and both `create-agent --agent-gid GID` and `serve --agent-gid GID`. The broker rejects a same-UID agent connection, so a second LaunchAgent under the owner's login is not a production substitute.

Controlled checkout also adds these absolute `ProgramArguments`: `--checkout-runtime-dir`, `--checkout-profiles-dir`, `--node-path`, and `--adapter-script`. The runtime and profile directories remain `0700` under the owner identity. Do not share them with the agent group. Run `./scripts/setup-owner` first to build the adapter and initialize those directories, then copy its printed absolute paths into the reviewed service definition.

Switching the provider to manual mode after daemon startup does not create a grace period: the broker checks current provider mode and peer UID on every agent request. A same-UID integration that worked in the simulator stops working immediately. Restart with the external group-shared socket layout before connecting a real card.

## Linux systemd

Use a dedicated service account and a private directory. An owner-reviewed unit can use:

```ini
[Service]
ExecStart=/absolute/path/target/release/cixa serve --data-dir /var/lib/cixa
User=cixa
Group=cixa
UMask=0077
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=strict
ProtectHome=yes
ReadWritePaths=/var/lib/cixa
```

The agent should connect through only the bounded `cixa.sock` endpoint or a brokered IPC proxy, not receive `owner.sock` or read access to `/var/lib/cixa`. Mount the owner socket only into the owner CLI or dashboard identity so agent connection flooding cannot consume owner-control admission.

For manual-provider mode, create a dedicated `treasury-agent-ipc` group and an agent service account. The broker owner must be permitted to assign that group, but the agent account must be a distinct UID. Put the shared socket and agent token outside the `0700` broker data directory, then run:

```bash
cixa create-agent --data-dir /var/lib/cixa \
  --owner-token-file /var/lib/cixa/owner.token \
  --agent-token-file /run/cixa-agent/agent.token \
  --agent-gid "$(getent group treasury-agent-ipc | cut -d: -f3)"
cixa serve --data-dir /var/lib/cixa \
  --socket /run/cixa-agent/cixa.sock \
  --agent-gid "$(getent group treasury-agent-ipc | cut -d: -f3)" \
  --checkout-runtime-dir /var/lib/cixa/checkout-runtime \
  --checkout-profiles-dir /var/lib/cixa/checkout-profiles \
  --node-path /usr/bin/node \
  --adapter-script /opt/cixa/packages/checkout-playwright/dist/index.js
```

`create-agent` writes, syncs, permissions, and syncs the token directory before it asks the broker to activate that capability. If activation fails or the broker response is lost, Cixa retains the named token file and reports an uncertain outcome. Reconcile the Agents view and that exact file before removing it or trying another creation.

The owner console supports the same boundary. Start it with `--agent-token-directory /run/cixa-agent/tokens --agent-gid GID`, or pass `--agent-gid` and `--agent-directory` to `scripts/setup-owner` and use its printed command. New token directories become `0750` and tokens `0640`, owned by the owner and shared only to that group before capability activation.

The broker changes only the agent token and agent socket to that group. `owner.sock`, `owner.token`, `audit.key`, and state remain private. Manual-provider startup rejects a missing or primary `--agent-gid`, and the socket rejects peers using the broker UID even if they can reach it.

The browser executable, Node binary, adapter script, runtime directory, and profiles are part of the owner-controlled trusted computing base. The daemon validates absolute regular paths, ownership, permissions, and non-writable parent chains before checkout. Keep package updates and browser updates owner-reviewed, then rerun synthetic merchant tests.

## Windows

The current reference binary intentionally fails closed on Windows because it does not yet ship a named-pipe implementation. Do not substitute a public TCP listener. A Windows service adapter must bind a named pipe with a DACL granting only the broker service and explicitly authorized agent identity, use the same v1 envelope, and keep the data directory and Credential Manager access owner-only. This limitation is a release gate for Windows rather than a hidden unsafe fallback.

## Custom Containerized Agent

Start from the supplied `agent` image target whenever possible. For a custom agent image, use a read-only root filesystem, no Docker socket, no browser remote-debugging port, and only the read-only `cixa-agent-ipc` volume. The shopping agent may need ordinary egress, but its Cixa MCP sidecar does not and should use `network_mode: none`. Keep the broker in the supplied owner service or a separate owner account. The owner dashboard remains on loopback and is never mounted into the agent container.

For Codex CLI, Claude Code, or another general coding agent, `./scripts/cixa-docker agent-config TOKEN_FILENAME` prints a ready-to-paste MCP entry. Install the `cixa-payments` skill on the agent host, mount only `/run/cixa-agent/cixa.sock` and its capability token, and leave owner data, the dashboard session, browser profile, and checkout runtime absent. A skill can teach correct behavior, but it cannot replace this isolation.

## Owner Dashboard

The dashboard requires two private files: the broker owner token, which remains server-side, and a separate dashboard access token entered into the local unlock screen. Generate the latter under `umask 077`, pass it with `--access-token-file`, and never mount it into the agent sandbox. The HTML and bundled static assets contain no private data and load before unlock. The access token is exchanged once for an HTTP-only, random, per-process session plus a readable CSRF cookie, then cleared from the form. API data and owner controls require that session, the Host allowlist, and exact Origin and CSRF checks. A session captured after a loopback port takeover is invalid against a restarted dashboard.

The Provider tab may launch `secret-session`. The card form is sent only to the loopback server, piped to the helper, and cleared. The helper child is terminated when the owner ends the session or the dashboard stops. `--cixa-binary`, `--checkout-runtime-directory`, and `--checkout-profiles-directory` should match the daemon's absolute owner-only paths. `scripts/setup-owner` also installs a private Playwright Chromium and passes it through `--checkout-browser-executable`, which pre-fills new merchant profiles. Keep the runtime path short enough for a Unix socket. Stop the dashboard when it is not needed.

## TCP

No TCP mode is shipped. If a future deployment adds one, it must require explicit configuration, authenticated encryption, origin and replay protection, a warning, and a reviewed network threat model. A loopback bind without authenticated encryption is not a substitute for the default Unix socket.

## Local Verification Policy

This solo-developer repository intentionally has no hosted GitHub Actions workflow. The owner runs `./scripts/verify` and `./scripts/verify-container` before a Docker image or release is published. This is an explicit resource and workflow choice, not evidence that hosted checks passed; a future multi-contributor release should reassess hosted branch protection, image signing, provenance attestations, and independent build infrastructure.

## Durable Record Quotas

Cixa accepts at most 16 agent records, 8 direct merchant approvals per agent, 10,000 purchase intents in one treasury, and 2,000 intents per agent. A revoked or expired record can be rotated in place to a fresh approval-required capability without consuming another agent slot. Policies are limited to 2 KiB. Ordinary audit actions stop 1,024 entries before the hard audit ceiling so provider outcomes, quarantine, restart recovery, handoff completion, and reconciliation retain capacity. Stop/resume transitions use a separate bounded reserve of 256 transitions, repeated no-op requests are suppressed, and a final stop remains available after that reserve is exhausted. Purchase requests are limited to 4 KiB and individual redirect URLs to 2 KiB. Requests above an agent's per-minute rate are rejected before an intent is stored, with one coalesced audit event per minute. These limits keep a compromised agent from growing `state.json` or dashboard responses without bound. Before a treasury reaches a quota, stop agents, keep an owner-only backup of the complete data directory and audit export, then initialize a new data directory and issue new capabilities. Do not delete individual records from an active treasury because that invalidates its authenticated state and audit chain.
