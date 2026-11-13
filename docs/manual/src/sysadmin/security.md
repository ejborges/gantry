---
title: Security and incidents
summary: The protections that are on by default, how to harden further, and what to do when something looks wrong.
---

## What protects the system by default

| Area | Protection |
| --- | --- |
| Transport | nginx serves HTTPS only and redirects HTTP; Gantry sends HSTS when its public address is `https`. Protocols and ciphers follow the system crypto policy. |
| Sign-in | Argon2id password hashes; account lockout; a limit on failed sign-ins per address; identical answers for unknown accounts and wrong passwords. Single sign-on with OpenID Connect (PKCE, state and nonce, asymmetric signatures only) or SAML 2.0 (signature checked against the certificates you configured, audience, recipient, validity period and one-time use; see [what Gantry checks](../admin/sign-in.md#what-gantry-checks)). |
| Sessions | Random 256-bit tokens, stored hashed; cookies are `HttpOnly`, `SameSite=Lax` and `Secure`; idle and absolute time limits; ended on password change and deactivation. |
| Forged requests | Every change needs the session's anti-forgery token and must come from the configured origin. No cross-origin access is granted. |
| API tokens | Stored hashed, shown once, scoped, expiring, individually revocable, tied to an account that can be deactivated. |
| Authorization | Deny by default; checked on the server for every request; invisible objects answer *not found*. Links, mentions and embedded cards inside text are resolved for each reader, so text never reveals a title or a status the reader may not see. |
| AI agents | A separate kind of account, marked everywhere, that cannot sign in interactively and is allowed nothing until an administrator writes its rules. Rules narrow the agent's roles; a token narrows them further. See [AI agent accounts](../admin/agents.md). |
| Content | User text is sanitized before display; a strict content security policy forbids inline script and framing by other sites; pictures, sound and video are loaded only from Gantry itself, so text cannot make a reader's browser contact another server. Attached pictures, recordings and PDF files are displayed; every other attachment is downloaded; and every attachment, whatever its type, is served with a policy that forbids it to run script. Custom scripts and HTML run only in a sealed frame: see [Custom content and scripts](#custom-content-and-scripts). |
| Live connection | The WebSocket is accepted only from the application's own origin for browser sessions, authorizes each subscription like the corresponding read and checks it again every minute, carries change hints and never content, and limits connections per account and messages per second. |
| Outbound calls | Every address the server is asked to contact (integrations, single sign-on discovery and metadata, token and key endpoints) is resolved and checked; private and loopback addresses are refused unless you allow them; redirects are not followed. |
| Email | Sent only to active accounts of people, only about things the recipient may open; names and titles cannot add headers to a message. |
| Inbound webhooks | Signature and timestamp verified; replays ignored. |
| Secrets at rest | Integration and single sign-on secrets encrypted with AES-256-GCM under `/etc/gantry/secret.key`. |
| Records | Append-only, hash-chained audit log; separate security event log. |
| Process | Unprivileged account; systemd sandbox; listens on loopback only. |

## Hardening checklist

- [ ] `server.public_url` is the `https` address and `gantry doctor` reports no failures.
- [ ] A certificate from your certificate authority replaces the self-signed one.
- [ ] Single sign-on is configured and `auth.allow_local_login = false`, with at least two system administrators able to sign in.
- [ ] SCIM deactivates leavers automatically.
- [ ] Only ports 80 and 443 are reachable from user networks; SSH from administration networks only.
- [ ] SELinux is enforcing. `gantry-setup` changes one boolean (`httpd_can_network_connect`) so that nginx can reach the application.
- [ ] `/etc/gantry` is mode 0750 root:gantry and the key 0640; backups are stored where only administrators can read them.
- [ ] The nightly backup is enabled and a restore has been tested.
- [ ] Operating system updates are applied monthly, or automatically with `dnf-automatic`.
- [ ] Logs are forwarded off the machine, and alerts exist for audit write failures and readiness.
- [ ] The built-in security alerts reach people who will act on them: **Administration**, **Alerts**; add a chat channel. The notification kind *Security incidents* is locked on.
- [ ] Every AI agent has rules that name its projects and spaces; none is listed with **No rules** without a reason; idle agent tokens are revoked.
- [ ] The permission to share dashboards and visualizations with the workspace (*analytics.share*) is held by the people who should have it.
- [ ] `gantry audit verify` runs on a schedule (below).

### Verify the audit log on a schedule

```bash
cat > /etc/systemd/system/gantry-audit-verify.service <<'UNIT'
[Unit]
Description=Verify the Gantry audit log hash chain
[Service]
Type=oneshot
ExecStart=/usr/bin/gantry audit verify
UNIT
cat > /etc/systemd/system/gantry-audit-verify.timer <<'UNIT'
[Timer]
OnCalendar=daily
Persistent=true
[Install]
WantedBy=timers.target
UNIT
systemctl daemon-reload && systemctl enable --now gantry-audit-verify.timer
```

The command exits non-zero and names the first altered entry if the chain is broken; alert on the unit failing.

## Seeing security incidents

Three places show the same events at different depths:

1. **Administration**, **Security events** in the console: counts by kind for the last 24 hours and the individual events. Two built-in alerts turn bursts of failed sign-ins and any critical event into notifications for system administrators: see [Alerts and monitoring](alerts.md#security-incidents).
2. The journal, target `security`: see [Logs](logs.md#a-security-event) for the fields and ready-made queries.
3. Metrics: `gantry_auth_failures_total` and `gantry_security_events_total` for alerting on rates.

## Responding

### Password guessing against one account

Signs: many `auth.login_failed` for one email, then `auth.lockout`.

The account locks itself for `auth.lockout_minutes`. Confirm with the owner that it was not them. If the attempts continue, block the source address at the firewall, and consider single sign-on with multi-factor.

### Password spraying from one address

Signs: `auth.login_failed` across many emails from one address, then `auth.login_throttled`.

```bash
journalctl -u gantry -o cat --since "1 hour ago" | jq -r 'select(.kind == "auth.login_failed") | .ip' | sort | uniq -c | sort -rn | head
firewall-cmd --permanent --add-rich-rule='rule family="ipv4" source address="203.0.113.9" reject' && firewall-cmd --reload
```

### A compromised account or token

1. In the console: **Users and agents**, **Manage**, **Sign out everywhere**, then **Deactivate** if in doubt. For an agent, revoke its tokens. Deactivation ends sessions and tokens immediately.
2. Establish what was done. In the audit log filter by that person and read from before the suspected time. For volume, query the database:

   ```bash
   runuser -u gantry -- psql gantry -c "SELECT at, auth_method, action, target_type, target_id, ip FROM audit_log WHERE actor_label = 'ada@example.org' AND at > now() - interval '7 days' ORDER BY at"
   ```
3. Reset the password or have the identity provider do so, then reactivate.
4. If the account could administer integrations or sign-in, rotate those secrets at the other systems and enter the new ones.

### Repeated permission denials

Signs: many `authz.denied` for one actor.

Usually a script with too little access. If it is a person probing, the detail shows what they tried. Talk to them; adjust roles or deactivate.

### Rejected webhook signatures

Signs: `webhook.signature_invalid` events.

Either the secret differs between the two systems, or something else is calling the address. The source address in the event tells you which.

### The audit chain does not verify

`gantry audit verify` reports the first entry that does not match and exits with an error. The same check run from the console (**Audit log**, **Verify integrity**) also records an `audit.chain_broken` security event of critical severity. This means the `audit_log` table was changed outside the application, which requires database superuser access. Treat it as a breach of the server: preserve the machine and the backups, compare with the forwarded journal lines (target `audit`) to find what was altered, and involve your security team.

### Suspected server compromise

Isolate the machine from the network, keep it running for forensics if your policy allows, and restore service on a new machine from a backup taken before the compromise. Afterward generate a new secret key, re-enter integration and single sign-on secrets, end all sessions (`TRUNCATE sessions`), and have all API tokens re-created.

## Custom content and scripts

Gantry lets people put their own HTML, SVG and chart scripts into dashboards, pages, messages and requests. This is the one place where code written by a user runs in other users' browsers, so it is contained in several layers. Understand them before deciding who may share scripts.

**How it is contained**

| Layer | Effect |
| --- | --- |
| The server never runs a script. | Nothing a user writes executes on the server. Scripts are stored as text. |
| A sandboxed frame without `allow-same-origin`. | The frame has an origin of its own that matches nothing: it cannot read the application's page, cookies or storage, cannot use the viewer's session, and cannot open windows, submit forms or navigate the page. |
| The frame's own content security policy: `default-src 'none'`, `connect-src 'none'`, `form-action 'none'`, `frame-src 'none'`, `worker-src 'none'`, pictures and fonts only from data embedded in the script. | The frame cannot make network requests: no fetch, no WebSocket, no beacons, no images or styles from other sites, no further frames. |
| Data only through the page that hosts the frame. | A script asks the hosting page for data; the page carries out a **GET** under `/api/v1/` with the **viewer's** session and hands back the JSON. Other methods are not carried out, and the server also refuses any change that is marked as coming from a script. |
| The application's policy allows frames only from itself. | A script cannot be loaded from, or replaced by, another site. |
| Charting libraries are served by Gantry. | Chart.js, Apache ECharts, Plotly and D3 are part of the package. Nothing is fetched from a content delivery network. |
| Sharing needs a permission, and every change is audited. | Sharing a visualization or dashboard with the workspace needs *analytics.share*. Creating and changing a script is recorded in the audit log with its author. |

**What this means**

- A script sees what its **viewer** may see through the API, never more. It runs with the viewer's access, so a script shared with the workspace reads data as each person who opens it.
- A script cannot change anything.
- A script has no channel to send what it reads to another server.

**What remains**

No browser sandbox is absolute, and you should know the limits:

- The policy `connect-src 'none'` covers every request API. Browsers have had ways around it that are not requests in that sense: WebRTC connection attempts and DNS lookups triggered by hints in markup. Gantry removes the WebRTC interfaces inside the frame before any content runs and sends the `webrtc 'block'` policy directive for browsers that support it; neither is a guarantee on every browser version. A determined author with permission to share could conceivably leak small amounts of what a viewer can read through such a side channel.
- A script can show misleading content inside its own tile, such as a fake message. It cannot draw outside its tile or over the application.
- A script can use the viewer's processor and memory while its tile is open.

So treat *analytics.share* as **the permission to run code in colleagues' browsers with read access to what they can see**, and give it accordingly:

- In an organization where members trust one another it can stay with the Member role, as shipped.
- Where projects hold information that some members must not be able to obtain from others, remove *analytics.share* from the Member role (use a custom role) and give it to people who may publish scripts. People can still write and use private visualizations, which run only in their own browser.
- Review changes to shared scripts: filter the audit log by `visualization.` and `dashboard.`.
- Blocks of custom HTML or script inside a page, message or request (`embed` and `viz` blocks) run under the same containment for everyone who can read that text. Who can write there is governed by the ordinary project and space roles.

Setting `[realtime] enabled = false` does not affect any of this. There is no setting that lets scripts run outside the frame.

## What is tested

Each release is tested against misuse as well as use. The test suites are part of the source and run in the build; the security-relevant ones cover:

| Area | What the tests do |
| --- | --- |
| Access | Every private route refuses anonymous callers. Members of one project try every route with the identifiers of another project's items, pages, files, comments, teams, dashboards and searches, and must learn nothing and change nothing. Roles, token scopes and privilege escalation are exercised directly. |
| AI agents | An agent without rules can do nothing by any route; an agent allowed only to read cannot change anything by any route, through the API or MCP; MCP offers each caller only the tools it may use. |
| Injection | A corpus of hostile strings is sent in every text field, query parameter, identifier, sort key and search criterion: SQL (`' OR '1'='1`, `; DROP TABLE`, time-based probes), path traversal, script and markup, template and shell syntax, header splitting, control and direction-override characters, absurd numbers and very long input. The server must answer quickly with a normal response or a clear refusal, never an internal error, a stall or a description of its insides, and the hostile text must come back as text. |
| Requests from other sites | A signed-in browser session cannot be used from another origin to read or to change, including over the WebSocket. Changes need the session's anti-forgery token. |
| Files | Uploaded files cannot run as pages; files outside the served folders cannot be read through the web application, the manual or attachment names. |
| XML | SAML and imported XML cannot name external files or entities, or expand without bound. Altered, re-wrapped, replayed, expired and wrongly addressed SAML responses are refused. |
| Headers and mail | Names, titles and file names cannot add headers to responses or to email. |
| Limits | Oversized, malformed and expensive requests are refused; sign-in is throttled and locks out; request rates are limited; the live connection withstands floods and nonsense. |
| Outbound | Integration and single sign-on addresses cannot reach internal services. |
| Records | Every change is audited with its author; the audit chain detects alteration; secrets never appear in logs or responses. |
| The browser | Hostile text in titles, descriptions, messages, labels, pages and administration screens is displayed as text; the content security policy stops injected script; a sandboxed script cannot reach the application, the network or the viewer's session. |

Tests show the presence of protection in the cases they cover; they do not prove the absence of flaws. Keep the installation updated and report what you find.

## Reporting a vulnerability

If you find a security problem in Gantry itself, report it privately to the maintainers of your distribution of Gantry rather than in a public tracker.
