// Screenshots used in the manual. With dev/run-local.sh running:
//   node web/scripts/manual-shots.cjs docs/manual/src/img
const { chromium } = require('playwright');
const out = process.argv[2];
const BASE = require('./base-url.cjs').baseUrl();
(async () => {
  const browser = await chromium.launch();
  const open = async (email) => {
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2, colorScheme: 'light' });
    const page = await ctx.newPage();
    await page.goto(BASE + '/login');
    await page.waitForSelector('input[name=email]');
    if (email === null) { await page.screenshot({ path: `${out}/login.png` }); await ctx.close(); return null; }
    await page.fill('input[name=email]', email);
    await page.fill('input[name=password]', 'correct horse battery staple');
    await page.click('button:has-text("Sign in")');
    await page.waitForSelector('button[aria-label*="Account menu"]');
    return { ctx, page };
  };
  await open(null);
  const shot = async (page, name, path, prepare) => {
    await page.goto(BASE + path);
    await page.waitForLoadState('networkidle');
    if (prepare) await prepare(page);
    await page.waitForTimeout(400);
    await page.screenshot({ path: `${out}/${name}.png` });
  };
  const admin = await open('admin@example.org');
  await shot(admin.page, 'board', '/projects/PUMP?view=board');
  await shot(admin.page, 'item', '/i/PUMP-2');
  await shot(admin.page, 'queues', '/projects/IT?view=queues');
  await shot(admin.page, 'docs', '/docs/ENG', async (p) => { await p.click('.tree a:has-text("Release process")'); await p.waitForSelector('.mermaid svg', { timeout: 20000 }); });
  // 0.2.0: discussions, the editor, planning, analytics, people.
  const region = async (page, name, path, selector, prepare) => {
    await page.goto(BASE + path);
    await page.waitForLoadState('networkidle');
    if (prepare) await prepare(page);
    await page.waitForTimeout(500);
    const el = page.locator(selector).first();
    await el.scrollIntoViewIfNeeded();
    await el.screenshot({ path: `${out}/${name}.png` });
  };
  const api = async (page, path) => page.evaluate(async (p) => (await fetch(p)).json(), path);
  await shot(admin.page, 'discussion', '/i/PUMP-2', async (p) => {
    const box = p.locator('.discussion .cm-content').last();
    await box.scrollIntoViewIfNeeded();
    await box.click();
    await p.keyboard.type('Soak test passed on both boards. ');
    await p.keyboard.type('@ada');
    await p.getByRole('option').first().click().catch(() => p.keyboard.press('Enter'));
    await p.keyboard.type(' can you review before PUMP-4 lands?');
    await p.locator('.discussion').getByRole('group', { name: 'What to show' }).getByRole('button', { name: 'Both' }).click();
    await p.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
    await p.locator('main').evaluate((m) => m.scrollTo?.(0, m.scrollHeight));
    await p.waitForTimeout(1800); // the result resolves mentions and links a moment after typing stops
  });
  await shot(admin.page, 'editor', '/i/PUMP-2', async (p) => {
    await p.locator('section, div').filter({ has: p.getByRole('heading', { name: 'Description', exact: true }) }).last().getByRole('button', { name: 'Edit' }).click();
    await p.locator('.cm-content').first().waitFor();
    await p.getByRole('group', { name: 'What to show' }).first().getByRole('button', { name: 'Both' }).click();
    await p.evaluate(() => window.scrollTo(0, 0));
  });
  await shot(admin.page, 'review', '/docs/ENG', async (p) => {
    await p.click('.tree a:has-text("Release process")');
    await p.getByRole('link', { name: 'Edit' }).click();
    await p.getByRole('group', { name: 'Editor' }).getByRole('button', { name: /Advanced/ }).click();
    const source = p.locator('.cm-content').first();
    await source.click();
    await p.keyboard.press('Control+End');
    await p.keyboard.type('\n\n## Rollback\n\nIf the smoke test fails, redeploy the previous tag and open an incident.\n');
    await p.getByRole('button', { name: 'Review and save' }).click();
    await p.getByRole('dialog').waitFor();
  });
  await shot(admin.page, 'diagram', '/docs/ENG', async (p) => {
    await p.click('.tree a:has-text("Weekly status")');
    await p.getByRole('link', { name: 'Edit' }).click();
    await p.getByRole('group', { name: 'Editor' }).getByRole('button', { name: /Advanced/ }).click();
    await p.locator('.cm-content').first().click();
    await p.keyboard.press('Control+End');
    await p.keyboard.press('Enter');
    await p.getByRole('button', { name: 'Draw a diagram' }).click();
    const d = p.getByRole('dialog');
    await d.waitFor();
    for (const [shape, text] of [['Data store', 'Request log'], ['Note', 'Page the engineer on call outside working hours']]) {
      await d.getByRole('toolbar', { name: 'Add a shape' }).getByRole('button', { name: shape, exact: true }).click();
      await d.getByLabel('Text').fill(text);
    }
    await p.waitForTimeout(500);
  });
  await shot(admin.page, 'planning', '/planning');
  await shot(admin.page, 'graph', '/planning/graph', async (p) => {
    await p.locator('.graph-node').first().waitFor({ timeout: 20000 });
    const fold = p.getByRole('button', { name: /Show the \d+ beneath PUMP/ });
    if (await fold.count()) await fold.first().click();
    await p.waitForTimeout(600);
  });
  const containers = await api(admin.page, '/api/v1/containers');
  const program = (containers.containers ?? containers).find((c) => c.name === 'Plant modernization');
  await shot(admin.page, 'report', `/planning/${program.id}`, async (p) => { await p.getByRole('tab', { name: 'Report' }).click(); await p.waitForLoadState('networkidle'); });
  await shot(admin.page, 'gantt', '/planning/gantt', async (p) => { await p.getByRole('link', { name: /Reliability plan/ }).click(); await p.locator('.gantt-bar').first().waitFor({ timeout: 20000 }); });
  await shot(admin.page, 'workflow', '/projects/PUMP?view=settings', async (p) => {
    await p.getByRole('button', { name: 'Design its own' }).first().click();
    await p.getByRole('dialog').locator('.react-flow__node').first().waitFor({ timeout: 20000 });
    await p.waitForTimeout(800);
  });
  await shot(admin.page, 'dashboard', '/analytics', async (p) => {
    await p.getByRole('link', { name: /Controller program at a glance/ }).click();
    await p.locator('.dash-tile').first().waitFor({ timeout: 20000 });
    await p.waitForTimeout(6000); // the sandboxed charts draw after their data arrives
  });
  const find = encodeURIComponent(JSON.stringify({ c: [['project', 'in', ['PUMP']], ['status_category', 'not_in', ['done']], ['due', 'before', { days: 30 }]], s: [['due', 'asc']] }));
  await shot(admin.page, 'find', `/search?find=${find}&open=PUMP-2`, async (p) => {
    await p.getByRole('region', { name: /^Work item / }).waitFor({ timeout: 20000 }).catch(() => {});
  });
  const teams = await api(admin.page, '/api/v1/teams');
  const firmware = (teams.teams ?? teams).find((t) => t.name === 'Firmware');
  await shot(admin.page, 'team', `/teams/${firmware.id}`);
  await shot(admin.page, 'org-chart', '/teams/chart');
  const users = await api(admin.page, '/api/v1/admin/users?limit=200');
  const agent = (users.users ?? users.items ?? users).find((u) => u.kind === 'agent');
  await shot(admin.page, 'agent-rules', `/admin/agents/${agent.id}`);
  await shot(admin.page, 'admin', '/admin/overview');
  await shot(admin.page, 'schemes', '/admin/schemes');
  await shot(admin.page, 'audit', '/admin/audit');
  await admin.ctx.close();
  const ada = await open('ada@example.org');
  await shot(ada.page, 'home', '/');
  await ada.ctx.close();
  const pat = await open('pat@example.org');
  await shot(pat.page, 'portal', '/portal');
  await pat.ctx.close();
  await browser.close();
})();
