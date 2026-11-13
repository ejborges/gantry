#!/usr/bin/env python3
"""Fill a fresh development server with realistic sample data through the public API."""
import json, sys, urllib.request, http.cookiejar

BASE = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:8600"
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
admin.call("POST", "/api/v1/projects", {"key": "ORB", "name": "Orbiter demonstration mission", "template": "nasa-flight-project", "access": "internal"})
admin.call("PUT", "/api/v1/admin/settings/site", {"name": "Gantry", "announcement": "", "support": {"name": "Operations desk", "email": "ops-desk@example.org", "url": "", "text": "Weekdays 08:00 to 17:00. For anything urgent, call extension 4100."}})

# ───── What 0.2 added: profiles, an AI agent with rules, teams, planning, charts ─────
import datetime

today = datetime.date.today()
day = lambda n: (today + datetime.timedelta(days=n)).isoformat()

for email, title in [("ada@example.org", "Firmware engineer"), ("ben@example.org", "Service desk lead"), ("chen@example.org", "Test engineer")]:
    admin.call("PATCH", f"/api/v1/admin/users/{people[email]}", {"job_title": title, "org_name": "Northfield Controls", "org_code": "NFC-ENG"})
admin.call("PATCH", "/api/v1/auth/me", {"job_title": "Engineering manager", "org_name": "Northfield Controls"})
admin.call("PATCH", f"/api/v1/admin/users/{me}", {"handle": "morgan"})
Client("ben@example.org").call("PATCH", "/api/v1/auth/me", {"presence": "busy", "status_message": "On the service desk until noon"})
Client("chen@example.org").call("PATCH", "/api/v1/auth/me", {"presence": "out_of_office", "status_message": "Back on Monday"})

# The agent may read, comment and work on items, in PUMP only. Without rules it can do nothing.
pump_id = admin.call("GET", "/api/v1/projects/PUMP")["project"]["id"]
admin.call("PUT", f"/api/v1/admin/users/{agent}/agent-policy", {
    "mode": "restricted", "all_projects": False, "projects": [pump_id], "all_spaces": False, "spaces": [], "rate_limit_per_minute": 60,
    "permissions": ["workspace.access", "project.view", "item.comment", "item.create", "item.edit", "item.transition", "item.assign"]})
admin.call("PATCH", "/api/v1/items/PUMP-9", {"assignee_id": agent})

# Dates, so the Gantt chart and the reports have something to show.
plan = {1: (-30, 45), 2: (-12, -5), 3: (-6, 6), 4: (-9, -5), 5: (2, 16), 6: (-25, -14), 7: (-3, 9), 8: (5, 23), 9: (8, 12), 10: (20, 34)}
for n, (start, due) in plan.items():
    admin.call("PATCH", f"/api/v1/items/PUMP-{n}", {"start_date": day(start), "due_date": day(due)})
for n in (3, 4, 5, 7):
    admin.call("PATCH", f"/api/v1/items/PUMP-{n}", {"parent": "PUMP-1"})
admin.call("POST", "/api/v1/projects/PUMP/cycles", {"name": "Sprint 14", "start_date": day(-7), "end_date": day(6), "goal": "Stop the resets at Site B"})
ada = Client("ada@example.org")
ada.call("POST", "/api/v1/items/PUMP-2/comments", {"body": "@morgan the fix is on the bench rig. Plan:\n\n- [x] swap C14 on two rev C boards\n- [ ] soak test for 72 hours, done by [[" + day(4) + "]]\n\n> [!warning]\n> PUMP-4 has to land first.\n\n| Board | Resets in 12 h |\n|---|---|\n| rev C, old C14 | 17 |\n| rev C, new C14 | 0 |"})

# The levels above projects, and what is on them.
levels = admin.call("PUT", "/api/v1/hierarchy/levels", {"kind": "container", "levels": [{"name": "Program", "plural": "Programs", "color": "purple"}, {"name": "Initiative", "plural": "Initiatives", "color": "teal"}]})
program = admin.call("POST", "/api/v1/containers", {"level_id": levels[0]["id"], "name": "Plant modernization", "status": "active", "start_date": day(-90), "target_date": day(240), "description": "Replace the controllers at all three sites and retire the legacy panels."})
reliability = admin.call("POST", "/api/v1/containers", {"level_id": levels[1]["id"], "parent_id": program["id"], "name": "Controller reliability", "status": "active", "start_date": day(-40), "target_date": day(60)})
flight = admin.call("POST", "/api/v1/containers", {"level_id": levels[1]["id"], "parent_id": program["id"], "name": "Flight demonstration", "status": "planned", "target_date": day(200)})
admin.call("PATCH", f"/api/v1/containers/{reliability['id']}", {"owner_id": people["ada@example.org"]})
admin.call("PUT", "/api/v1/projects/PUMP/container", {"container_id": reliability["id"]})
admin.call("PUT", "/api/v1/projects/ORB/container", {"container_id": flight["id"]})
admin.call("PATCH", "/api/v1/projects/PUMP", {"lead_id": people["ada@example.org"]})

# Teams: a team is its positions; people and agents fill them.
eng = admin.call("POST", "/api/v1/teams", {"name": "Engineering", "description": "Everyone who designs, builds and tests the controllers."})
fw = admin.call("POST", "/api/v1/teams", {"name": "Firmware", "parent_id": eng["id"], "description": "Controller firmware and its tools."})
desk_team = admin.call("POST", "/api/v1/teams", {"name": "Service desk", "parent_id": eng["id"]})


def position(team, title, who, lead=False, since=-200):
    # A new team already has a "Team lead" position, held by whoever created it.
    detail = admin.call("GET", f"/api/v1/teams/{team['id']}")
    if not any(p["title"] == title for p in detail["positions"]):
        detail = admin.call("POST", f"/api/v1/teams/{team['id']}/positions", {"title": title, "is_lead": lead})
    pos = next(p for p in detail["positions"] if p["title"] == title)
    for uid in who:
        admin.call("POST", f"/api/v1/teams/{team['id']}/positions/{pos['id']}/assignments", {"user_id": uid, "started_on": day(since)})


position(fw, "Team lead", [people["ada@example.org"]], lead=True, since=-400)
position(fw, "Firmware engineer", [me, people["chen@example.org"]], since=-250)
position(fw, "Triage", [agent], since=-20)
position(desk_team, "Team lead", [people["ben@example.org"]], lead=True, since=-600)

# A Gantt chart with a deadline line, a shaded span and a group; a saved search; a dashboard.
admin.call("POST", "/api/v1/gantt", {"name": "Reliability plan", "scope": {"project": "PUMP"}, "shared": True, "settings": {"zoom": "week", "group_by": "none", "color_by": "status"},
    "annotations": [{"id": "a1", "type": "line", "date": day(14), "color": "red", "label": "Site B shutdown", "style": "dashed"},
                    {"id": "a2", "type": "range", "start": day(-7), "end": day(6), "color": "amber", "label": "Bench rig booked"},
                    {"id": "a3", "type": "group", "items": ["PUMP-2", "PUMP-4"], "color": "blue", "label": "Reset fix"}]})
admin.call("POST", "/api/v1/searches", {"name": "Late or critical, mine", "shared": False, "alert": True,
    "query": {"match": "any", "criteria": [{"field": "overdue", "op": "is", "value": True}, {"field": "priority", "op": "gte", "value": 4}], "sort": [{"field": "due", "dir": "asc"}]}})
dash = admin.call("POST", "/api/v1/dashboards", {"name": "Controller program at a glance", "description": "Open work, flow and workload across the firmware project.", "visibility": "workspace", "params": {"project": ""}})
tiles = [("kpi", "kpi-tiles", 0, 0, 12, 2), ("status", "status-donut", 0, 2, 4, 4), ("flow", "throughput-bars", 4, 2, 8, 4), ("load", "workload-stacked", 0, 6, 6, 5), ("due", "due-table", 6, 6, 6, 5), ("cycle", "cycle-scatter", 0, 11, 6, 5), ("types", "type-treemap", 6, 11, 6, 5)]
for tid, ref, x, y, w, h in tiles:
    admin.call("PUT", f"/api/v1/dashboards/{dash['id']}/tiles/{tid}", {"kind": "template", "ref": ref, "x": x, "y": y, "w": w, "h": h})
viz = admin.call("POST", "/api/v1/visualizations", {"template": "priority-bars", "name": "Open firmware work by priority", "visibility": "workspace", "params": {"project": "PUMP"}})
admin.call("POST", "/api/v1/spaces/ENG/pages", {"title": "Weekly status", "editor": "advanced", "content": "## Where we stand\n\nThe reset fix (PUMP-2) is on the bench. Open work by priority:\n\n![[viz:" + viz["id"] + "|300]]\n\n```diagram\n{\"nodes\":[{\"id\":\"a\",\"x\":20,\"y\":20,\"w\":140,\"h\":52,\"shape\":\"round\",\"text\":\"Bench fix\",\"color\":\"green\"},{\"id\":\"b\",\"x\":230,\"y\":20,\"w\":140,\"h\":52,\"shape\":\"rect\",\"text\":\"72 h soak test\",\"color\":\"blue\"},{\"id\":\"c\",\"x\":440,\"y\":14,\"w\":150,\"h\":64,\"shape\":\"diamond\",\"text\":\"No resets?\",\"color\":\"amber\"}],\"edges\":[{\"id\":\"e1\",\"from\":\"a\",\"to\":\"b\",\"label\":\"\",\"style\":\"solid\",\"arrow\":\"end\"},{\"id\":\"e2\",\"from\":\"b\",\"to\":\"c\",\"label\":\"\",\"style\":\"solid\",\"arrow\":\"end\"}]}\n```\n"})
print("seeded:", ", ".join(["PUMP", "IT", "ORB", "space ENG", f"{len(people)} people", "1 agent with rules", "1 program, 2 initiatives", "3 teams", "1 Gantt chart", "1 dashboard"]))
