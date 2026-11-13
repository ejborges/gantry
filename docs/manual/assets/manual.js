// Search box for the manual. The index is a small JSON file built with the pages.
(() => {
  const input = document.getElementById("q");
  const list = document.getElementById("results");
  if (!input || !list) return;
  const base = document.body.dataset.base || "";
  let index = null;
  const load = () => (index ??= fetch(base + "search.json").then((r) => r.json()).catch(() => []));
  const show = (items) => {
    list.replaceChildren(
      ...items.map((d) => {
        const li = document.createElement("li");
        const a = document.createElement("a");
        a.href = base + d.url;
        const title = document.createElement("strong");
        title.textContent = d.title;
        const where = document.createElement("span");
        where.textContent = d.section;
        a.append(title, where);
        li.append(a);
        return li;
      }),
    );
    if (!items.length) {
      const li = document.createElement("li");
      li.className = "none";
      li.textContent = "Nothing found";
      list.append(li);
    }
    list.hidden = false;
  };
  input.addEventListener("focus", load);
  input.addEventListener("input", async () => {
    const words = input.value.toLowerCase().split(/\s+/).filter(Boolean);
    if (!words.length) return void (list.hidden = true);
    const docs = await load();
    const scored = docs
      .map((d) => {
        const title = d.title.toLowerCase();
        const heads = d.headings.join(" ").toLowerCase();
        const text = d.text.toLowerCase();
        let score = 0;
        for (const w of words) {
          if (title.includes(w)) score += 10;
          else if (heads.includes(w)) score += 4;
          else if (text.includes(w)) score += 1;
          else return null;
        }
        return { d, score };
      })
      .filter(Boolean)
      .sort((a, b) => b.score - a.score)
      .slice(0, 8);
    show(scored.map((s) => s.d));
  });
  // "/" jumps to the search box from anywhere that is not a text field.
  document.addEventListener("keydown", (e) => {
    const t = e.target;
    if (e.key !== "/" || e.ctrlKey || e.metaKey || e.altKey || (t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName)))) return;
    e.preventDefault();
    input.focus();
    input.select();
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Escape") (list.hidden = true), input.blur();
    if (e.key === "Enter") list.querySelector("a")?.click();
  });
  document.addEventListener("click", (e) => {
    if (!e.target.closest(".search")) list.hidden = true;
  });
})();

// Theme: follow the device, or light, or dark. Remembered in this browser, and shared with
// the application when both come from the same address.
(() => {
  const box = document.querySelector(".theme");
  if (!box) return;
  const read = () => {
    try {
      const t = localStorage.getItem("gantry.theme");
      return t === "light" || t === "dark" ? t : "system";
    } catch {
      return "system";
    }
  };
  const apply = (choice) => {
    if (choice === "system") delete document.documentElement.dataset.theme;
    else document.documentElement.dataset.theme = choice;
    for (const b of box.querySelectorAll("button")) b.setAttribute("aria-pressed", String(b.dataset.choice === choice));
  };
  box.addEventListener("click", (e) => {
    const b = e.target.closest("button[data-choice]");
    if (!b) return;
    try {
      localStorage.setItem("gantry.theme", b.dataset.choice);
    } catch {
      /* the choice holds for this page only */
    }
    apply(b.dataset.choice);
  });
  // Changed in another tab, or in the application: follow along.
  window.addEventListener("storage", (e) => e.key === "gantry.theme" && apply(read()));
  box.hidden = false;
  apply(read());
})();
