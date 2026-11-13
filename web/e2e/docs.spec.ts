import { expect, test } from "@playwright/test";
import { Api, MEMBER, signInAs, uniq } from "./helpers";

test.describe("documentation", () => {
  test("write a page in the simple editor, extend it in the advanced editor, and review its history", async ({ page, baseURL }) => {
    const api = await Api.as(baseURL!, MEMBER);
    const key = `D${uniq()}`;
    await api.ok("POST", "/api/v1/spaces", { key, name: `Handbook ${key}` });
    await signInAs(page, MEMBER);
    await page.goto(`/docs/${key}/new`);
    await expect(page.getByRole("heading", { name: "New page" })).toBeVisible();
    await page.getByLabel("Title").fill("Commissioning procedure");

    // Simple editor: type, then make a word bold with the toolbar.
    const editor = page.locator(".ProseMirror");
    await editor.click();
    await page.keyboard.type("Isolate the pump before opening the housing. ");
    await page.getByRole("toolbar", { name: "Formatting" }).getByRole("button", { name: "Bold" }).click();
    await page.keyboard.type("Lock out the breaker.");
    await page.getByRole("button", { name: "Publish page" }).click();
    await expect(page.getByRole("heading", { name: "Commissioning procedure" })).toBeVisible();
    await expect(page.locator(".prose strong")).toHaveText("Lock out the breaker.");

    // Advanced editor: Markdown source with a live preview, and a diagram.
    await page.getByRole("link", { name: "Edit" }).click();
    await page.getByRole("group", { name: "Editor" }).getByRole("button", { name: /Advanced/ }).click();
    const source = page.locator(".cm-content");
    await source.click();
    await page.keyboard.press("Control+End");
    await page.keyboard.type("\n\n## Sequence\n\n```mermaid\ngraph LR\nIsolate --> Open --> Inspect\n");
    await page.getByRole("group", { name: "What to show" }).getByRole("button", { name: "Both" }).click();
    await expect(page.getByRole("region", { name: "Page content: how it will read" }).getByRole("heading", { name: "Sequence" })).toBeVisible();
    // Before anything is published, the changes are shown side by side with the saved version.
    await page.getByRole("button", { name: "Review and save" }).click();
    const review = page.getByRole("dialog", { name: "Review your changes" });
    await expect(review.getByText("Version 1 (saved)")).toBeVisible();
    await expect(review.locator(".diff .add, .diff ins").first()).toBeVisible();
    await expect(review.getByText("## Sequence")).toBeVisible();
    // Side by side: the added lines are in the right-hand column, in line with the unchanged lines around them.
    const rightNumbers = await review.locator(".diff-table tbody tr").evaluateAll((rows) =>
      rows
        .filter((r) => r.querySelector("td.diff-n") && !r.lastElementChild!.classList.contains("diff-none"))
        .map((r) => Math.round([...r.querySelectorAll("td.diff-n")].pop()!.getBoundingClientRect().x)),
    );
    expect(rightNumbers.length).toBeGreaterThan(3);
    expect(new Set(rightNumbers).size, `line numbers of the new version at ${[...new Set(rightNumbers)]}`).toBe(1);
    const table = await review.locator(".diff-table").boundingBox();
    expect(rightNumbers[0]).toBeGreaterThan(table!.x + table!.width * 0.45);
    await review.getByRole("group", { name: "Layout" }).getByRole("button", { name: "Inline" }).click();
    await expect(review.locator(".diff-table.inline")).toBeVisible();
    await expect(review.getByText("## Sequence")).toBeVisible();
    await review.getByRole("group", { name: "Layout" }).getByRole("button", { name: "Side by side" }).click();
    await review.getByLabel("What changed").fill("Added the sequence diagram");
    await review.getByRole("button", { name: "Save and publish this version" }).click();
    await expect(page.locator(".prose .mermaid svg")).toBeVisible({ timeout: 20_000 });

    // A page with a diagram can only be edited in the advanced editor, so nothing is lost.
    await page.getByRole("link", { name: "Edit" }).click();
    await expect(page.getByRole("group", { name: "Editor" }).getByRole("button", { name: /Simple/ })).toBeDisabled();
    await page.goBack();

    await page.getByRole("link", { name: /History/ }).click();
    await expect(page.getByText("Added the sequence diagram")).toBeVisible();
    const rows = page.locator("tbody tr");
    await expect(rows).toHaveCount(2);
    await expect(rows.first()).toContainText("Current");
    await expect(rows.first()).toContainText("Ada Okafor");
    await api.dispose();
  });

  test("draw a diagram, put it in a page, and change it later", async ({ page, baseURL }) => {
    const failures: string[] = [];
    page.on("pageerror", (e) => failures.push(String(e)));
    const api = await Api.as(baseURL!, MEMBER);
    const key = `G${uniq()}`;
    await api.ok("POST", "/api/v1/spaces", { key, name: `Drawings ${key}` });
    const created = await api.ok("POST", `/api/v1/spaces/${key}/pages`, { title: "Intake flow", content: "How a request is handled.\n", status: "published" });
    await signInAs(page, MEMBER);
    await page.goto(`/docs/${key}/${created.id}/edit`);
    await page.getByRole("group", { name: "Editor" }).getByRole("button", { name: /Advanced/ }).click();
    await page.locator(".cm-content").click();
    await page.keyboard.press("Control+End");
    await page.keyboard.press("Enter");

    // A new diagram starts from a small example. Add three shapes, each given its words in the side panel.
    await page.getByRole("button", { name: "Draw a diagram" }).click();
    const dialog = page.getByRole("dialog", { name: "Draw a diagram" });
    await expect(dialog.locator(".react-flow__node")).toHaveCount(3);
    const shapes = dialog.getByRole("toolbar", { name: "Add a shape" });
    for (const [shape, text] of [["Start or end", "Request arrives"], ["Decision", "Urgent?"], ["Step", "Page the engineer on call"]]) {
      await shapes.getByRole("button", { name: shape, exact: true }).click();
      await dialog.getByLabel("Text").fill(text);
    }
    await expect(dialog.locator(".react-flow__node")).toHaveCount(6);
    // A new shape never lands on top of another one.
    await page.waitForTimeout(400);
    const boxes = await dialog.locator(".react-flow__node").evaluateAll((els) => els.map((e) => e.getBoundingClientRect()).map((r) => ({ x: r.x, y: r.y, w: r.width, h: r.height })));
    for (let i = 0; i < boxes.length; i++)
      for (let j = i + 1; j < boxes.length; j++) {
        const [a, b] = [boxes[i], boxes[j]];
        expect(a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h, `shapes ${i} and ${j} overlap`).toBe(false);
      }
    // Changing a shape after the first one is what used to bring the page down.
    await dialog.getByLabel("Shape", { exact: true }).selectOption({ label: "State" });
    await dialog.getByRole("radio", { name: "green" }).click();
    await dialog.getByRole("button", { name: "Insert diagram" }).click();
    await expect(dialog).toBeHidden();

    // It is part of the text, and the result shows the drawing.
    await expect(page.locator(".cm-content")).toContainText("```diagram");
    await page.getByRole("group", { name: "What to show" }).getByRole("button", { name: "Both" }).click();
    const drawn = page.locator(".md-preview .embed-diagram svg, .preview .embed-diagram svg, .embed-diagram svg").first();
    await expect(drawn).toBeVisible();
    await expect(page.locator(".embed-diagram").first()).toContainText("Request arrives");

    // With the cursor in the block, the same button opens the diagram again.
    await page.locator(".cm-content").getByText("```diagram").click();
    await page.keyboard.press("ArrowDown");
    await page.getByRole("button", { name: "Draw a diagram" }).click();
    const again = page.getByRole("dialog", { name: "Edit diagram" });
    await expect(again.locator(".react-flow__node")).toHaveCount(6);
    await again.locator(".react-flow__node").filter({ hasText: "Urgent?" }).click();
    // The editor behind the dialog had its cursor where a long line wraps. It used to pull
    // keyboard focus back a moment after losing it, and what was typed here went into the page.
    await again.getByLabel("Text").focus();
    await page.waitForTimeout(400);
    await expect(again.getByLabel("Text")).toBeFocused();
    await again.getByLabel("Text").fill("Priority?");
    await expect(again.getByLabel("Text")).toHaveValue("Priority?");
    // Tab stays inside the dialog.
    for (let i = 0; i < 30; i++) await page.keyboard.press("Tab");
    expect(await page.evaluate(() => !!document.activeElement?.closest('[role="dialog"][aria-modal="true"]'))).toBe(true);
    await expect(page.locator(".cm-content")).not.toContainText("Priority?");
    await again.getByRole("button", { name: "Update diagram" }).click();
    await expect(page.locator(".embed-diagram").first()).toContainText("Priority?");
    await expect(page.locator(".cm-content").getByText("```diagram")).toHaveCount(1);

    // Saved with the page, and drawn for readers.
    await page.getByRole("button", { name: "Review and save" }).click();
    await page.getByRole("dialog", { name: "Review your changes" }).getByRole("button", { name: "Save and publish this version" }).click();
    await expect(page.getByRole("heading", { name: "Intake flow" })).toBeVisible();
    await expect(page.locator(".prose .embed-diagram svg")).toBeVisible();
    // Long text is wrapped onto lines inside its shape.
    await expect(page.locator(".prose .embed-diagram")).toContainText("Page the engineer");
    const stored = await api.ok("GET", `/api/v1/pages/${created.id}`);
    expect(stored.content).toContain("```diagram");
    expect(stored.content).toContain("Priority?");
    expect(stored.content).toContain("Page the engineer on call");
    expect(failures).toEqual([]);
    await api.dispose();
  });

  test("text typed and not saved waits for the next editor, and can be discarded", async ({ page, baseURL }) => {
    const api = await Api.as(baseURL!, MEMBER);
    const key = `U${uniq()}`;
    await api.ok("POST", "/api/v1/spaces", { key, name: `Drafts ${key}` });
    const created = await api.ok("POST", `/api/v1/spaces/${key}/pages`, { title: "Calibration", content: "Saved text.\n", status: "published" });
    await signInAs(page, MEMBER);
    await page.goto(`/docs/${key}/${created.id}/edit`);
    const editor = page.locator(".ProseMirror");
    await expect(editor).toContainText("Saved text.");
    await editor.click();
    await page.keyboard.press("Control+End");
    await page.keyboard.type(" Half a thought.");
    await expect(page.getByText("There are changes that are not saved.")).toBeVisible();
    // Leaving without saving keeps the draft: the page itself is unchanged...
    await page.getByRole("link", { name: "Cancel" }).click();
    await expect(page.locator(".prose")).toContainText("Saved text.");
    await expect(page.locator(".prose")).not.toContainText("Half a thought.");
    // ...and the editor continues where it was left.
    await page.getByRole("link", { name: "Edit" }).click();
    await expect(page.locator(".ProseMirror")).toContainText("Half a thought.");
    await expect(page.getByRole("button", { name: "Review and save" })).toBeEnabled();

    // Discarding returns to the saved version, after asking.
    await page.getByRole("button", { name: "Discard changes" }).click();
    const confirm = page.getByRole("dialog", { name: "Discard the unsaved changes?" });
    await expect(confirm).toContainText("Everything typed since version 1 was saved is removed");
    await confirm.getByRole("button", { name: "Discard changes" }).click();
    await expect(page.locator(".prose")).toContainText("Saved text.");
    await page.getByRole("link", { name: "Edit" }).click();
    await expect(page.locator(".ProseMirror")).toContainText("Saved text.");
    await expect(page.locator(".ProseMirror")).not.toContainText("Half a thought.");
    await expect(page.getByRole("button", { name: "Review and save" })).toBeDisabled();
    await expect(page.getByText("There are changes that are not saved.")).toHaveCount(0);
    expect((await api.ok("GET", `/api/v1/pages/${created.id}`)).version).toBe(1);
    await api.dispose();
  });

  test("two people editing the same page: the second save is told, not overwritten", async ({ page, baseURL }) => {
    const api = await Api.as(baseURL!, MEMBER);
    const key = `C${uniq()}`;
    await api.ok("POST", "/api/v1/spaces", { key, name: `Shared ${key}` });
    const created = await api.ok("POST", `/api/v1/spaces/${key}/pages`, { title: "Shift handover", content: "Original text.\n", status: "published" });
    await signInAs(page, MEMBER);
    await page.goto(`/docs/${key}/${created.id}/edit`);
    await page.getByRole("group", { name: "Editor" }).getByRole("button", { name: /Advanced/ }).click();
    await page.locator(".cm-content").click();
    await page.keyboard.type("My edit. ");
    // Someone else saves first.
    await api.ok("PATCH", `/api/v1/pages/${created.id}`, { content: "Their edit.\n", base_version: 1 });
    await page.getByRole("button", { name: "Review and save" }).click();
    await page.getByRole("dialog", { name: "Review your changes" }).getByRole("button", { name: "Save and publish this version" }).click();
    await expect(page.getByText(/Your text is still here/)).toBeVisible();
    await expect(page.locator(".cm-content")).toContainText("My edit.");
    const now = await api.ok("GET", `/api/v1/pages/${created.id}`);
    expect(now.content).toContain("Their edit.");
    expect(now.content).not.toContain("My edit.");
    await api.dispose();
  });
});
