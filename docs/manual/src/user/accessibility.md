---
title: Keyboard and accessibility
nav: Keyboard and accessibility
summary: Using Gantry without a mouse, with a screen reader, at high zoom, in high contrast or with less motion, and what to do when something gets in your way.
---

Gantry is built to the Web Content Accessibility Guidelines (WCAG) 2.1, level AA. Everything can be done with the keyboard, every control has a name a screen reader can say, and nothing is told by color alone. This page says how, feature by feature.

## Moving around with the keyboard

| To | Press |
| --- | --- |
| Jump past the navigation to the page itself | <kbd>Tab</kbd> once after a page opens, then <kbd>Enter</kbd> on **Skip to content** |
| Open search from anywhere | <kbd>Ctrl</kbd> <kbd>K</kbd> (<kbd>⌘</kbd> <kbd>K</kbd> on a Mac) |
| Move through a menu | <kbd>↓</kbd> <kbd>↑</kbd>, <kbd>Home</kbd>, <kbd>End</kbd>; <kbd>Esc</kbd> closes it and returns to its button |
| Close a dialog | <kbd>Esc</kbd>. Focus returns to the control that opened it. |
| Leave a text editor | <kbd>Esc</kbd>, then <kbd>Tab</kbd> (or <kbd>Shift</kbd> <kbd>Tab</kbd> to go back) |

Whatever has keyboard focus has a ring drawn around it. A dialog puts focus in its first field when it opens; while it is open, <kbd>Tab</kbd> moves among the dialog's own controls and does not wander into the page behind it.

> **Note**: In the Markdown editor <kbd>Tab</kbd> indents the line, as it does in any text editor, so it does not move on. Press <kbd>Esc</kbd> and then <kbd>Tab</kbd> to leave the editor. A screen reader announces this with the editor.

## Things that are usually done by dragging

Every one of them has another way.

| Where | Without dragging |
| --- | --- |
| Board: move a card | <kbd>Tab</kbd> to the card, <kbd>Tab</kbd> once more to its **Move to…** list, and choose. Or open the item and use the status buttons. |
| Gantt chart: change dates | Focus a bar: <kbd>←</kbd> <kbd>→</kbd> move it, <kbd>Shift</kbd> with the arrows changes the due date, <kbd>Alt</kbd> with the arrows the start. Or type the dates on the item. See [Gantt charts](gantt.md). |
| Workflow editor | The tables under the canvas hold every stage and every move, with the same controls. See [Workflows](workflows.md). |
| Diagram editor | <kbd>Tab</kbd> moves between shapes, the arrow keys move the selected one, and the panel beside the canvas changes its text, shape and color. |
| Dashboard: arrange tiles | In **Edit** mode, focus a tile's handle: the arrow keys move it and <kbd>Shift</kbd> with the arrows changes its size. See [Analytics](analytics.md). |
| Hierarchy graph | Every node is a button: <kbd>Tab</kbd> reaches it and <kbd>Enter</kbd> opens its details. The **Structure** tab shows the programs, initiatives and projects as a list. |
| Attach a file | The **Attach files** button opens your device's file chooser. |
| Choose a date in the editor | **Insert a date**, <kbd>Tab</kbd> to the days, then the arrow keys (<kbd>Page Up</kbd> and <kbd>Page Down</kbd> change the month) and <kbd>Enter</kbd>. |

## With a screen reader

- Every page has one main heading and landmarks for the navigation and the content. The browser tab is named after the page, for example *Planning · Gantry*.
- Fields are labeled, and a field's hint is read with it. Errors are announced when they appear.
- **People and AI agents are told apart in words**, not only by the shape of the picture: an agent's name is followed by *AI agent* wherever it appears, including in mentions and in lists to choose from.
- A person's status is part of their picture's name, for example *Ada Okafor, Away*.
- Status, priority and service targets are written out; the color is extra.
- What changes by itself is announced without taking focus: a notification that arrives, a message someone posts while you read a discussion, that someone is writing, who else has the page open and what they are editing, and the wait message of a slow page.
- Charts in reports come with **The numbers as a table**. Built-in visualizations describe themselves; for your own, see the checklist in [Visualizations](../reference/visualizations.md#accessibility-checklist).
- Dates and times are real dates to the browser, shown in the time zone from your profile.

## Seeing the page

| You want | How |
| --- | --- |
| A dark or light page | Account menu, **Theme**: **Match device**, **Light** or **Dark**. Both meet the contrast level of WCAG AA. This manual has the same switch at the top of each page. |
| Larger text | Zoom with your browser. Pages rearrange themselves down to the width of a phone, which is what a laptop window gives at 300%, and do not scroll sideways. |
| High contrast | Your system's high contrast mode (forced colors) is respected. Bars on charts and the status rings in the graph keep their own colors there, because the color is what they show. |
| Less motion | Turn on *reduce motion* in your device's settings. Gantry then stops moving placeholders, slides and smooth scrolling; a waiting mark fades where it would turn. |

## On a phone or tablet

Controls are sized for a finger and the navigation moves to a bar at the bottom of the screen. Tables keep status and priority in view on a tablet and become lists on a phone. Nothing needs a hover or a drag: see the table above.

## What is checked, and what is not

Each release is tested with an automated checker (axe-core, WCAG 2.1 A and AA rules) on every kind of page in both themes, and with keyboard-only tests of the skip link, menus, dialogs, the editor, the board and the calendar. Automated checks find only part of what matters. Content that people write is theirs to make accessible. A picture you attach is inserted with its file name as its description, `![pump-curve.png](…)`: replace the name with a few words that say what the picture shows. For your own visualizations, follow the checklist linked above.

If something cannot be reached, read or used, tell your administrator, who can reach the people who run Gantry in your organization: see [When something goes wrong](troubleshooting.md).
