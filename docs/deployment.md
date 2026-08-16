# Local Deployment

The broker and the agent should run as separate OS identities or containers. The agent receives only the scoped interface and token file. It must not mount the broker data directory, secret-helper socket, owner dashboard session, raw audit files, or browser debugging ports.

## macOS launchd

Create an owner-reviewed `~/Library/LaunchAgents/com.example.agent-treasury.plist` with absolute paths and a private data directory. The important arguments are:

```xml
<key>ProgramArguments</key>
<array>
  <string>/absolute/path/target/release/treasury</string>
  <string>serve</string>
  <string>--data-dir</string>
  <string>/Users/OWNER/.local/agent-treasury</string>
</array>
<key>RunAtLoad</key><true/>
<key>Umask</key><integer>63</integer>
<key>StandardOutPath</key><string>/Users/OWNER/.local/agent-treasury/daemon.out</string>
<key>StandardErrorPath</key><string>/Users/OWNER/.local/agent-treasury/daemon.err</string>
```

Review the plist before `launchctl bootstrap gui/$UID ...`. Do not put a token or card secret in the plist.

This per-user LaunchAgent is for simulated development only. Manual-provider mode requires a broker daemon and agent process under distinct macOS UIDs, a dedicated shared IPC group, agent token and socket paths outside the broker's private directory, and both `create-agent --agent-gid GID` and `serve --agent-gid GID`. The broker rejects a same-UID agent connection, so a second LaunchAgent under the owner's login is not a production substitute.

## Linux systemd

Use a dedicated service account and a private directory. An owner-reviewed unit can use:

```ini
[Service]
ExecStart=/absolute/path/target/release/treasury serve --data-dir /var/lib/agent-treasury
User=agent-treasury
Group=agent-treasury
UMask=0077
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=strict
ProtectHome=yes
ReadWritePaths=/var/lib/agent-treasury
```

The agent should connect through only the bounded `treasury.sock` endpoint or a brokered IPC proxy, not receive `owner.sock` or read access to `/var/lib/agent-treasury`. Mount the owner socket only into the owner CLI or dashboard identity so agent connection flooding cannot consume owner-control admission.

For manual-provider mode, create a dedicated `treasury-agent-ipc` group and an agent service account. The broker owner must be permitted to assign that group, but the agent account must be a distinct UID. Put the shared socket and agent token outside the `0700` broker data directory, then run:

```bash
treasury create-agent --data-dir /var/lib/agent-treasury \
  --owner-token-file /var/lib/agent-treasury/owner.token \
  --agent-token-file /run/agent-treasury-agent/agent.token \
  --agent-gid "$(getent group treasury-agent-ipc | cut -d: -f3)"
treasury serve --data-dir /var/lib/agent-treasury \
  --socket /run/agent-treasury-agent/treasury.sock \
  --agent-gid "$(getent group treasury-agent-ipc | cut -d: -f3)"
```

The broker changes only the agent token and agent socket to that group. `owner.sock`, `owner.token`, `audit.key`, and state remain private. Manual-provider startup rejects a missing or primary `--agent-gid`, and the socket rejects peers using the broker UID even if they can reach it.

## Windows

The current reference binary intentionally fails closed on Windows because it does not yet ship a named-pipe implementation. Do not substitute a public TCP listener. A Windows service adapter must bind a named pipe with a DACL granting only the broker service and explicitly authorized agent identity, use the same v1 envelope, and keep the data directory and Credential Manager access owner-only. This limitation is a release gate for Windows rather than a hidden unsafe fallback.

## Containerized Agent

Run the agent with a read-only root filesystem, no host network, no Docker socket, no browser remote-debugging port, and only a narrow socket proxy or mounted token file. Keep the broker on the host or a separate service account. The owner dashboard remains on loopback and is never mounted into the agent container.

## Owner Dashboard

The dashboard requires two private files: the broker owner token, which remains server-side, and a separate dashboard access token that the owner enters through the browser's HTTP Basic prompt. Generate the latter under `umask 077`, pass it with `--access-token-file`, and never mount it into the agent sandbox. Authentication is required before the HTML, static assets, status API, or emergency endpoint are served. The HTTP-only session cookie, readable CSRF cookie, Host allowlist, and exact Origin check are secondary controls. Stop the dashboard when it is not needed.

## TCP

No TCP mode is shipped. If a future deployment adds one, it must require explicit configuration, authenticated encryption, origin and replay protection, a warning, and a reviewed network threat model. A loopback bind without authenticated encryption is not a substitute for the default Unix socket.

## Local Verification Policy

This solo-developer repository intentionally has no hosted GitHub Actions workflow. The owner runs `./scripts/verify` locally before each push. This is an explicit resource and workflow choice, not evidence that hosted checks passed; a future multi-contributor or public release should reassess hosted branch protection and independent build infrastructure.
