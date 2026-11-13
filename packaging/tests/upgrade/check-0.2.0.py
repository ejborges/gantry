#!/usr/bin/env python3
"""After a 0.1.0 database has been migrated by 0.2.0: is everything still there, and did
what the upgrade notes promise happen? (Manual: System administration, Upgrading.)

    check-0.2.0.py BASE_URL IDS_FILE

Exits 1 if any check fails. Used by packaging/tests/upgrade.sh."""
import json, sys, urllib.request, http.cookiejar
BASE = sys.argv[1]; ids = json.load(open(sys.argv[2]))
jar = http.cookiejar.CookieJar(); op = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
def call(method, path, body=None, headers=None, raw=False):
    req = urllib.request.Request(BASE + path, method=method, data=None if body is None else json.dumps(body).encode())
    req.add_header("Content-Type", "application/json")
    for k, v in (headers or {}).items(): req.add_header(k, v)
    try:
        with (op if not (headers and "Authorization" in headers) else urllib.request.build_opener()).open(req) as r:
            t = r.read(); return (r.status, t if raw else (json.loads(t) if t else None))
    except urllib.error.HTTPError as e:
        return (e.code, e.read().decode()[:200])
ok = True
def check(name, cond, detail=""):
    global ok; ok = ok and bool(cond); print(("ok   " if cond else "FAIL ") + name + (f": {detail}" if detail else ""))
s, _ = call("POST", "/api/v1/auth/login", {"email": "admin@example.org", "password": "correct horse battery staple"}); check("the administrator signs in with the same password", s == 200, s)
s, me = call("GET", "/api/v1/auth/me"); csrf = me["csrf"]; H = {"X-Gantry-CSRF": csrf}
check("accounts have a handle and a time zone setting", me["user"].get("handle"), me["user"].get("handle"))
s, b1 = call("GET", "/api/v1/items/BUY-1"); check("the canceled order has the new status key", s == 200 and b1["status"] == "canceled", b1.get("status") if s == 200 else b1)
s, hist = call("GET", "/api/v1/items/BUY-1/history"); check("its history reads with the new key and no old one", s == 200 and "cancelled" not in json.dumps(hist) and "canceled" in json.dumps(hist), s)
s, b2 = call("GET", "/api/v1/items/BUY-2"); tr = [t["to"] for t in b2.get("transitions", [])] if s == 200 else []
check("an order that was open can still be canceled", b2["status"] == "canceled" or ("canceled" in tr and "cancelled" not in tr), tr)
s, r = call("POST", "/api/v1/items/BUY-2/transition", {"to": "canceled"}, H); check("…and canceling it works", s == 200 or b2["status"] == "canceled", s if s != 200 else "")
s, p2 = call("GET", "/api/v1/items/PUMP-2"); check("work, descriptions, links and labels are intact", s == 200 and p2["labels"] == ["firmware", "site-b"] and "40 minutes" in (p2.get("description") or "") and p2.get("parent"), p2.get("labels") if s == 200 else p2)
s, c = call("GET", "/api/v1/items/PUMP-2/comments"); n = len(c if isinstance(c, list) else c.get("comments", c.get("messages", []))) if s == 200 else -1
check("the discussion is intact", n >= 2, n)
s, it1 = call("GET", "/api/v1/items/IT-1"); check("the service desk request kept its status and assignee", s == 200 and it1["status"] == "waiting_customer" and it1.get("assignee"), it1.get("status") if s == 200 else it1)
s, pg = call("GET", f"/api/v1/pages/{ids['page']}"); check("pages are intact", s == 200 and "mermaid" in pg["content"], s)
s, se = call("GET", "/api/v1/search?q=capacitor"); check("search finds old work", s == 200 and any("C14" in json.dumps(x) for x in se.get("results", [])), s)
s, users = call("GET", "/api/v1/admin/users"); ul = users["users"] if isinstance(users, dict) else users
ag = next(u for u in ul if u["id"] == ids["agent"])
check("every account has a handle", all(u.get("handle") for u in ul), [u.get("handle") for u in ul])
s, pol = call("GET", f"/api/v1/admin/users/{ids['agent']}/agent-policy")
check("the agent from before the upgrade is unrestricted until rules are written", s == 200 and (pol.get("policy", pol)).get("mode") == "unrestricted", pol if s != 200 else (pol.get("policy", pol)).get("mode"))
A = {"Authorization": "Bearer " + ids["token"]}
s, _ = call("GET", "/api/v1/items/PUMP-2", headers=A); check("its token still reads", s == 200, s)
s, _ = call("POST", "/api/v1/items/PUMP-2/comments", {"body": "Triage note after the upgrade."}, A); check("…and still writes, as before", s in (200, 201), s)
s, av = call("GET", f"/api/v1/users/{ids['agent']}/avatar", raw=True); check("accounts without a picture get a generated one", s == 200 and b"<svg" in av[:200], s)
s, al = call("GET", "/api/v1/admin/alerts"); rules = al.get("rules", al) if s == 200 else []
check("the built-in alerts were created", s == 200 and len(rules) >= 8 and all(r.get("builtin") for r in rules), len(rules) if s == 200 else al)
s, pk = call("GET", "/api/v1/admin/packs"); packs = (pk.get("packs") or pk.get("installed") or []) if isinstance(pk, dict) else pk
core = next((p for p in packs if p.get("id") == "core"), {}) if s == 200 else {}
check("the built-in definitions are those of the new version", core.get("version", "").startswith("0.2."), core.get("version") if s == 200 else pk)
s, tp = call("GET", "/api/v1/project-templates"); names = [t["name"] for t in (tp if isinstance(tp, list) else tp.get("templates", []))]
check("built-in names follow the new version", "Research program" in names and "Research programme" not in names, [n for n in names if "esearch" in n])
s, _ = call("GET", "/api/v1/catalog"); s2, _ = call("GET", "/api/v1/catalogue"); check("the catalog has its new address only", s == 200 and s2 == 404, (s, s2))
s, ig = call("GET", "/api/v1/admin/integrations"); il = ig.get("integrations", ig) if s == 200 else []
check("the integration and its settings are kept", s == 200 and len(il) == 1 and il[0]["name"] == "Build server", s)
s, lv = call("GET", "/api/v1/containers"); check("planning starts with the default levels and nothing placed", s == 200 and len(lv.get("levels", [])) >= 2 and len(lv.get("unplaced_projects", [])) == 3, (len(lv.get("levels", [])), len(lv.get("unplaced_projects", []))) if s == 200 else lv)
s, nt = call("GET", "/api/v1/notifications"); check("old notifications are still listed", s == 200, s)
print("All upgrade checks passed." if ok else "SOME UPGRADE CHECKS FAILED")
sys.exit(0 if ok else 1)
