#!/usr/bin/env python3
"""Fill a Gantry 0.1.0 server the way an installation in use would be filled, through its
public API: people, an AI agent with a role and a token, a software project, a service desk
with requests, a purchasing project with a canceled order, pages, and an integration.

    seed-0.1.0.py BASE_URL IDS_FILE

Writes what the checks after the upgrade need (ids, the agent's token) to IDS_FILE.
Used by packaging/tests/upgrade.sh."""
import json, sys, urllib.request, http.cookiejar

BASE = sys.argv[1]
PASSWORD = "correct horse battery staple"


class Client:
    def __init__(self, email):
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.jar))
        self.csrf = None
        self.call("POST", "/api/v1/auth/login", {"email": email, "password": PASSWORD})
        self.me = self.call("GET", "/api/v1/auth/me")
        self.csrf = self.me["csrf"]

    def call(self, method, path, body=None):
        req = urllib.request.Request(BASE + path, method=method, data=None if body is None else json.dumps(body).encode())
        req.add_header("Content-Type", "application/json")
        if self.csrf and method != "GET":
            req.add_header("X-Gantry-CSRF", self.csrf)
        try:
            with self.opener.open(req) as r:
                text = r.read().decode()
                return json.loads(text) if text else None
        except urllib.error.HTTPError as e:
            raise SystemExit(f"{method} {path} -> {e.code} {e.read().decode()}")


admin = Client("admin@example.org")
people = {}
for email, name, role in [("ada@example.org", "Ada Okafor", "member"), ("ben@example.org", "Ben Halvorsen", "member"), ("chen@example.org", "Chen Wei", "member"), ("pat@example.org", "Pat Quinn", "customer")]:
    people[email] = admin.call("POST", "/api/v1/admin/users", {"email": email, "name": name, "password": PASSWORD, "role": role})["id"]
agent = admin.call("POST", "/api/v1/admin/users", {"name": "Triage agent", "kind": "agent"})["id"]

# A software project with work in several states.
admin.call("POST", "/api/v1/projects", {"key": "PUMP", "name": "Pump controller firmware", "template": "software", "access": "internal", "description": "Firmware for the P-series pump controllers."})
for uid in [people["ada@example.org"], people["ben@example.org"], people["chen@example.org"], agent]:
    admin.call("POST", "/api/v1/projects/PUMP/members", {"user_id": uid, "role": "project_member"})
me = admin.me["user"]["id"]
items = [
    ("epic", "Controller reliability program", "todo", 3, None, {}),
    ("bug", "Controller resets under sustained load", "in_progress", 4, me, {"severity": "Critical", "environment": "Site B, firmware 4.1.2"}),
    ("story", "Operators can acknowledge alarms from the panel", "in_review", 2, people["ada@example.org"], {"story_points": 5}),
    ("task", "Replace capacitor C14 on rev C boards", "todo", 3, me, {}),
    ("bug", "Flow reading drifts after 72 hours", "backlog", 2, people["ben@example.org"], {"severity": "Major"}),
    ("story", "Export the event log as CSV", "done", 1, people["chen@example.org"], {"story_points": 3}),
    ("task", "Update the commissioning checklist", "in_progress", 2, people["ada@example.org"], {}),
    ("task", "Calibrate the bench flow meter", "todo", 2, me, {}),
    ("bug", "Display flickers below 5 °C", "todo", 1, None, {"severity": "Minor"}),
    ("story", "Dark mode for the operator panel", "backlog", 1, None, {}),
]
for i, (t, title, status, prio, assignee, custom) in enumerate(items, start=1):
    body = {"type": t, "title": title, "priority": prio, "custom": custom}
    if assignee:
        body["assignee_id"] = assignee
    if i in (2, 4):
        body["due_date"] = "2026-11-06"
    if i == 8:
        body["due_date"] = "2026-12-04"
    if i == 2:
        body["description"] = "The controller restarts roughly every **40 minutes** when the pump runs above 90 % duty.\n\n1. Run P-101 at 92 % for an hour\n2. Watch the watchdog counter\n\n| Board | Resets per hour |\n|---|---|\n| Rev B | 0 |\n| Rev C | 1.5 |\n"
    admin.call("POST", "/api/v1/projects/PUMP/items", body)
    key = f"PUMP-{i}"
    wf = {"todo": ["todo"], "in_progress": ["in_progress"], "in_review": ["in_review"], "done": ["done"], "backlog": []}
    if t == "task":
        wf = {"todo": [], "in_progress": ["in_progress"], "done": ["done"]}
    for to in wf.get(status, []):
        admin.call("POST", f"/api/v1/items/{key}/transition", {"to": to})
admin.call("PATCH", "/api/v1/items/PUMP-2", {"parent": "PUMP-1", "labels": ["firmware", "site-b"]})
admin.call("POST", "/api/v1/items/PUMP-4/links", {"rel": "blocks", "target": "PUMP-2"})
admin.call("POST", "/api/v1/items/PUMP-5/links", {"rel": "relates_to", "target": "PUMP-2"})
admin.call("POST", "/api/v1/items/PUMP-2/comments", {"body": "Reproduced on the bench with a rev C board. Scope trace attached to the lab notebook."})
Client("ada@example.org").call("POST", "/api/v1/items/PUMP-2/comments", {"body": "Rev B boards do not show it. I suspect the **C14** tolerance change."})

# A service desk with requests from a customer.
admin.call("POST", "/api/v1/projects", {"key": "IT", "name": "IT help", "template": "it-service-desk", "description": "Laptops, accounts, access and software."})
admin.call("POST", "/api/v1/projects/IT/members", {"user_id": people["ben@example.org"], "role": "service_agent"})
pat = Client("pat@example.org")
desk = pat.call("GET", "/api/v1/portal/desks/IT")
types = {t["name"]: t["id"] for t in desk["request_types"]}
pat.call("POST", "/api/v1/portal/desks/IT/requests", {"request_type_id": types["Something is broken"], "summary": "Laptop will not start after the update", "description": "Black screen with a blinking cursor since this morning.", "fields": {"impact": "High"}})
pat.call("POST", "/api/v1/portal/desks/IT/requests", {"request_type_id": types["Request access"], "summary": "Access to the test-stand data share", "fields": {"affected_system": "\\\\lab\\teststand"}})
pat.call("POST", "/api/v1/portal/desks/IT/requests", {"request_type_id": types["Ask a question"], "summary": "How do I book the calibration lab?"})
admin.call("PATCH", "/api/v1/items/IT-1", {"priority": 4, "assignee_id": me})
admin.call("POST", "/api/v1/items/IT-1/comments", {"body": "Likely the storage driver again; check BIOS version first.", "internal": True})
admin.call("POST", "/api/v1/items/IT-1/comments", {"body": "Please hold the power button for ten seconds, then start it again. Does the logo appear?"})
admin.call("POST", "/api/v1/items/IT-1/transition", {"to": "waiting_customer"})

# Documentation.
admin.call("POST", "/api/v1/spaces", {"key": "ENG", "name": "Engineering handbook", "access": "internal", "description": "How we design, test and release."})
home = admin.call("POST", "/api/v1/spaces/ENG/pages", {"title": "Release process", "editor": "advanced", "content": "Every firmware release follows the same path.\n\n```mermaid\nflowchart LR\n  A[Feature freeze] --> B[Bench test]\n  B --> C{Passes?}\n  C -- yes --> D[Site pilot]\n  C -- no --> A\n  D --> E[General release]\n```\n\n## Checklist\n\n- [x] Version number updated\n- [x] Change log reviewed\n- [ ] Pilot site notified\n\n## Sign-off\n\n| Role | Name | Date |\n|---|---|---|\n| Firmware lead | Ada Okafor | |\n| Quality | Chen Wei | |\n"})
admin.call("POST", "/api/v1/spaces/ENG/pages", {"title": "Bench test procedure", "template": "sop", "parent_id": home["id"]})
admin.call("POST", "/api/v1/spaces/ENG/pages", {"title": "Design review, 3 November", "template": "meeting-notes"})
admin.call("POST", "/api/v1/items/PUMP-2/links", {"rel": "documented_in", "target": {"type": "page", "id": home["id"]}})

# A NASA-style project from the add-on, and something in the security log.

# ── things an upgrade must carry over ──
schemes = admin.call("GET", "/api/v1/admin/schemes")
with_cancel = [(w["key"], [s["key"] for s in w["definition"]["statuses"]]) for w in schemes["workflows"] if any(s["key"] == "cancelled" for s in w["definition"]["statuses"])]
print("workflows with a 'cancelled' status:", [k for k, _ in with_cancel])
# The agent gets a workspace role and a token, as agents had in 0.1.0.
admin.call("POST", f"/api/v1/admin/users/{agent}/roles", {"role": "member"})
tok = admin.call("POST", f"/api/v1/admin/users/{agent}/tokens", {"name": "triage", "scopes": ["write"]})
open(sys.argv[2], "w").write(json.dumps({"agent": agent, "token": tok.get("token") or tok.get("secret"), "people": people, "page": home["id"]}))
# A purchasing project, whose built-in workflow had the status key "cancelled".
admin.call("POST", "/api/v1/projects", {"key": "BUY", "name": "Purchasing", "template": "procurement", "access": "internal"})
detail = admin.call("GET", "/api/v1/projects/BUY")
for t in detail["types"]:
    wf = t.get("workflow") or {}
    d = wf.get("definition") or wf
    if any(x["key"] == "cancelled" for x in d.get("statuses", [])):
        for n in range(2):
            it = admin.call("POST", "/api/v1/projects/BUY/items", {"type": t["key"], "title": f"Order that was {'canceled' if n == 0 else 'kept'} before the upgrade"})
            if n == 0:
                admin.call("POST", f"/api/v1/items/{it['key']}/transition", {"to": "cancelled"})
                print("canceled", it["key"], "type", t["key"])
        break
# A workflow the organization changed, and an integration with a secret.
admin.call("POST", "/api/v1/admin/integrations", {"kind": "webhook", "name": "Build server", "config": {"url": "https://ci.example.org/hooks/gantry"}, "secrets": {"signing_secret": "s3cr3t-signing-key"}, "events": ["item.created"]})
print("seeded")
