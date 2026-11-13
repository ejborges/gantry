# Research notes

Findings that shaped the design, gathered online on 2026-11-10. Items marked **verified** were
read at the linked source during this build; items marked **background** rest on general
engineering knowledge and were not re-checked.

## Integration and protocol facts (verified)

| Topic | Finding | Design consequence |
|---|---|---|
| Model Context Protocol | Current revision is `2026-07-28`. It is stateless: no `initialize` handshake, no `Mcp-Session-Id`; every request carries `io.modelcontextprotocol/protocolVersion` in `_meta`; servers must implement `server/discover`; every result carries `resultType`; list results carry `ttlMs` and `cacheScope`; HTTP POSTs must carry `MCP-Protocol-Version`, `Mcp-Method` and (for `tools/call`) `Mcp-Name`, validated against the body (`-32020 HeaderMismatch`, `-32022 UnsupportedProtocolVersion`). GET/DELETE on the endpoint return 405. | `/mcp` is a dual-era server: modern stateless requests and the legacy `initialize` flow (`2025-11-25`, `2025-06-18`, `2025-03-26`) are both served. |
| Microsoft Teams | Office 365 connectors (the old "incoming webhook" URLs) were shut off in Teams between May 18 and 22, 2026. The replacement is a Power Automate *Workflows* webhook ("When a Teams webhook request is received") that accepts Adaptive Cards. | The Teams integration posts Adaptive Card payloads to a Workflows URL. The legacy `MessageCard` format is not produced. |
| Jira Cloud | `GET/POST /rest/api/3/search` was removed in 2025. The replacement is `/rest/api/3/search/jql`, paged with `nextPageToken`, and fields must be requested explicitly. | The Jira importer uses `search/jql` for Cloud and `/rest/api/2/search` (`startAt`) for Data Center. |
| GitLab | GitLab 19.0/19.1 added HMAC *signing tokens* following the Standard Webhooks spec: headers `webhook-id`, `webhook-timestamp`, `webhook-signature` (`v1,<base64>`), signed string `{id}.{timestamp}.{body}`, key is the base64 payload after `whsec_`. The older plain `X-Gitlab-Token` is no longer recommended. | The GitLab receiver verifies the signature when present and falls back to a constant-time `X-Gitlab-Token` compare. Gantry's own outbound webhooks use the same Standard Webhooks scheme. |
| GitHub | Webhooks are signed with `X-Hub-Signature-256: sha256=<hex HMAC of body>`. (background) | Verified in constant time before the body is parsed. |
| Slack | Requests are signed `v0=HMAC-SHA256(secret, "v0:{timestamp}:{body}")` in `X-Slack-Signature`, with a five-minute replay window on `X-Slack-Request-Timestamp`. (background) | Slash-command endpoint enforces both. |

## Platform facts (verified)

| Topic | Finding | Design consequence |
|---|---|---|
| RHEL 10 application streams | PostgreSQL 16 (18 also available), Valkey 8.0 (Redis is gone), nginx 1.26, Node.js 22, Python 3.12, shipped as plain RPMs rather than modules. | Core RPMs depend on distro `postgresql-server`, `nginx`. The cache add-on targets Valkey, which speaks the Redis protocol. |
| pgvector on EL | `pgvector` 0.6.2 is in AppStream for EL 9.5+ (postgresql:16 module) and EL 10. | The vector add-on RPM requires `pgvector` and switches the vector backend from portable arrays to an HNSW index. |
| Object storage | MinIO's community edition went to maintenance mode in December 2025 and its repository was archived on April 25, 2026. Maintained S3-compatible options: SeaweedFS (Apache-2.0), Garage (AGPL-3.0), Ceph RGW; RustFS (Apache-2.0) is still pre-release. | The blob port has a filesystem backend and a generic S3 (SigV4, path-style) backend. Documentation recommends SeaweedFS or Garage, not MinIO. |
| Vagrant on VMware | Official AlmaLinux boxes support the `vmware_desktop` provider on x86_64 and aarch64 for AlmaLinux 9 and 10. | The Vagrantfile defaults to `almalinux/9` and lets `GANTRY_BOX` select another EL box. |

## NASA sources (verified)

- **NPR 7120.5F**, *NASA Space Flight Program and Project Management Requirements*. The copy on NODIS is
  published as NID 7120.148 (effective December 9, 2024). It stresses that tailoring is expected, adds leading
  indicators at life-cycle reviews and KDPs, and adjusts the required control plans.
- **NPR 7123.1D**, *NASA Systems Engineering Processes and Requirements*, effective July 5, 2023, expires July 5, 2028.
- **NASA Systems Engineering Handbook** (NASA/SP-2016-6105 Rev 2), section 3: phases Pre-Phase A to F and the
  reviews held in each (MCR; SRR, MDR/SDR; PDR; CDR, PRR, SIR; TRR, SAR, ORR, FRR/MRR; PLAR, CERR, PFAR, DR; DRR).

The NASA add-on encodes this structure as data (phases, key decision points, reviews, item types, templates).
It is a starting point to be tailored. It does not reproduce the directives, and anyone using it on a real
project must check the current NODIS revisions.

## Landscape (background, with sources consulted)

Open-source tools reviewed for patterns: Plane (AGPL-3.0), OpenProject (GPL-3.0), Redmine (GPL-2.0), Huly (EPL-2.0),
Taiga (MPL-2.0), Zammad (AGPL-3.0), Outline (BSL), Docmost (AGPL-3.0), BookStack (MIT), Wiki.js (AGPL-3.0).
None covers tracking, service desk and wiki in one RPM-installable server under a permissive license, and
copyleft code cannot be folded into an Apache-2.0 project, so Gantry is a new code base that composes
permissively licensed components:

| Need | Component | License |
|---|---|---|
| HTTP server | axum, tokio, tower | MIT |
| Relational, document (JSONB), queue, full-text, graph (recursive CTE) | PostgreSQL 15+ | PostgreSQL |
| Vector | float arrays in PostgreSQL, or pgvector | PostgreSQL |
| Search at scale (optional) | OpenSearch | Apache-2.0 |
| Objects at scale (optional) | SeaweedFS, Garage, Ceph RGW | Apache-2.0 / AGPL / LGPL (separate services) |
| Cache and shared counters (optional) | Valkey | BSD-3 |
| Front end | React, TanStack Query, TipTap (ProseMirror), CodeMirror 6, markdown-it, DOMPurify | MIT / Apache-2.0 |
| Reverse proxy | nginx | BSD-2 |

## Practices applied (background)

- **Tracking**: status *categories* (to do / in progress / done) separate from workflow status names, so boards,
  reports and automation work across differently named workflows; keys like `OPS-142`; configurable types,
  fields and transitions as data, not code.
- **Service desk**: request types in front of work item types; internal vs public comments; first-response
  and resolution targets per priority with business hours; knowledge suggestions before a request is filed
  (deflection, as in Knowledge-Centered Service).
- **Wiki**: Markdown as the single stored format so people, the two editors, the API and AI agents all read
  and write the same thing; full version history with restore.
- **Security**: OWASP ASVS 5.0 for sessions and access control; NIST SP 800-63B for passwords (length over
  composition rules, Argon2id); OAuth 2.0 Security BCP (RFC 9700) for OIDC (code flow, PKCE, state, nonce);
  deny by default with one authorization path for every route.
- **Logging**: OWASP Logging Cheat Sheet — structured events with who, what, when, where, outcome and a
  correlation ID; secrets never logged; audit trail tamper-evident (hash chain) and separate from debug logs.
- **UI**: dense but calm lists, color reserved for meaning (status category, priority, SLA state, destructive
  actions), keyboard access and a command palette, WCAG 2.2 AA contrast and focus visibility, layouts that
  reflow at tablet and phone widths.

## Sources

- [MCP changelog 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28/changelog)
- [MCP Streamable HTTP transport](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http.md)
- [MCP versioning and compatibility](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning.md)
- [MCP server/discover](https://modelcontextprotocol.io/specification/2026-07-28/server/discover)
- [Office 365 connectors retirement in Teams](https://o365reports.com/office-365-connectors-retirement-in-teams-and-migration-to-workflows/)
- [Jira JQL search migration guide](https://documentation.codefortynine.com/external-data-for-jira-fields/jql-search-migration-guide)
- [GitLab webhooks](https://docs.gitlab.com/user/project/integrations/webhooks/)
- [RHEL 10: languages, web servers, database servers](https://docs.redhat.com/en/documentation/red_hat_enterprise_linux/10/html/considerations_in_adopting_rhel_10/dynamic-programming-languages-web-servers-database-servers)
- [pgvector in Oracle Linux 9 AppStream](https://yum.oracle.com/repowatch/oraclelinux9.x86_64/ol9_appstream/5972b109b7de4ddf534a09d4351ff197e436b5f371867541cd07ef2375241ab8)
- [MinIO archived: self-hosted S3 alternatives](https://pinggy.io/blog/minio_archived_self_hosted_s3_alternatives/)
- [AlmaLinux Vagrant boxes](https://wiki.almalinux.org/installation/vagrant-boxes.html)
- [NID 7120.148 / NPR 7120.5F](https://nodis3.gsfc.nasa.gov/OPD_Docs/NID_7120_148_.pdf)
- [NASA Software Engineering Handbook reference to NPR 7123.1D](https://swehb.nasa.gov/x/EwH7)
- [NASA Systems Engineering Handbook, 3.0 Project Life Cycle](https://www.nasa.gov/seh/3-project-life-cycle)
- [Plane vs OpenProject (2026)](https://plane.so/blog/plane-vs-openproject-which-should-you-choose-in-2026)
- [OWASP Logging Vocabulary Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Logging_Vocabulary_Cheat_Sheet.html)
