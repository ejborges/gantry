---
title: Markdown reference
summary: Every piece of syntax the editor understands, from GitHub, GitLab and Obsidian, and what Gantry adds.
---

Descriptions, messages and pages are stored as Markdown text. Gantry reads the common ground of three dialects that people already write, so text pasted from GitHub, GitLab or an Obsidian vault usually reads as intended. The guide to the editor is [Writing](../user/writing.md).

## Basics (CommonMark)

| Write | Get |
| --- | --- |
| `# Heading` to `###### Heading` | Headings, levels 1 to 6. Each gets an anchor so you can link to a section. |
| `**bold**`, `*italic*`, `` `code` `` | Bold, italic, inline code |
| `[text](https://example.org)` | A link. Links that leave Gantry open in a new tab. |
| `- item`, `1. item` | Bulleted and numbered lists; indent to nest |
| `> quote` | A block quote |
| ` ``` ` … ` ``` ` | A code block. Name the language after the opening marks for highlighting: ` ```python `. |
| `---` on its own line | A horizontal rule |
| A line ending in two spaces, or `<br>` | A line break inside a paragraph |

## GitHub Flavored Markdown

| Write | Get |
| --- | --- |
| A row of cells between `\|` with a `\|---\|` line beneath the first row | A table. `:---`, `:---:` and `---:` align a column. |
| `- [ ]` and `- [x]` | A task list. The boxes show state; change them by editing the text. |
| `~~text~~` | Strikethrough |
| `https://example.org` | A bare address becomes a link |
| `text[^1]` and, elsewhere, `[^1]: The note.` | A footnote |
| `> [!NOTE]`, `> [!TIP]`, `> [!IMPORTANT]`, `> [!WARNING]`, `> [!CAUTION]` | An alert: a colored box with an icon and its kind as the title |
| `:tada:`, `:warning:` | Emoji by name |
| `<details><summary>More</summary> … </details>`, `<kbd>`, `<sub>`, `<sup>`, `<mark>`, `<abbr>`, `<ins>`, `<del>` | The safe subset of HTML. See [HTML](#html). |

## GitLab Flavored Markdown

| Write | Get |
| --- | --- |
| ``$`a^2 + b^2`$`` in a sentence; ` ```math ` for a block | A formula |
| ` ```mermaid ` | A diagram from text |
| `[[_TOC_]]` or `[TOC]` on its own line | A table of contents of the headings |
| `{+added+}` and `{-removed-}` | Text marked as added or removed |
| `>>>` on a line before and after several paragraphs | A block quote without a `>` on every line |
| `![clip](demo.mp4)`, `![memo](note.mp3)` | A video or audio player. See [Files](#files-and-media). |
| `![plan](plan.png){width=400}` or `{width=400 height=300}` | A picture at a given size |
| `` `#1f5fbf` `` | A color code followed by a swatch of the color |

## Obsidian Flavored Markdown

| Write | Get |
| --- | --- |
| `[[Target]]`, `[[Target\|shown text]]` | A link to a work item, project or page. See [Links to work](#links-to-work-people-and-dates). |
| `![[Target]]` | An embed: an attached picture or recording, a work item as a card, or a saved visualization |
| `![[photo.png\|300]]`, `![[photo.png\|300x200]]`, `![alt\|300](photo.png)` | A picture at a given width, or width and height |
| `==highlight==` | Highlighted text |
| `> [!note] Title` followed by quoted lines | A callout with your own title. Kinds: note, info, tip, hint, important, success, question, warning, caution, attention, danger, error, bug, example, quote, abstract, summary, todo. An unknown kind is shown as a note. |
| `%%hidden%%` | A comment: kept in the text, not shown to readers |
| `#tag`, `#area/pumps` | A tag |
| `---` … `---` at the very top | Properties (front matter): shown folded under **Properties** |
| `$x^2$`, `$$ … $$` | A formula in a sentence, or on its own line |

## Links to work, people and dates

| Write | Get | Rule |
| --- | --- | --- |
| `@ada.okafor` | A mention: a link to the profile, and a notification | You can mention accounts you can see; the person is told only if they can open the item or page. |
| `PUMP-12` | A link to the work item | Shown as a link only to readers who can open the item. |
| `[[PUMP-12]]`, `[[PUMP-12\|the seal problem]]` | A link showing key and title, or your words | Same. |
| `![[PUMP-12]]` | The item as a card with its status | Same. |
| `[[PUMP]]` | A link to the project | Same, for the project. |
| `[[Release process]]` | A link to the page with that title | Looked up in the same space first; elsewhere the title must be unique among published pages you can read. |
| `[[2026-03-31]]` | A date, written out in the reader's own style | A calendar day, the same for every reader. |
| `![[viz:` *id* `]]`, `![[viz:` *id* `\|320]]` | A saved visualization, optionally with a height in pixels | Shown to readers the visualization is shared with. |

A reference that cannot be resolved for the reader is shown as plain gray words with a dashed underline. It does not say whether the target exists.

References are not recognized inside code and inside ordinary links.

## Files and media

A file attached to the same work item, request or page can be named directly.

| Write | Get |
| --- | --- |
| `![wiring](wiring.png)` or `![[wiring.png]]` | The picture |
| `![run](run.mp4)` | A video player (`mp4`, `m4v`, `webm`, `ogv`, `mov`) |
| `![memo](memo.mp3)` | An audio player (`mp3`, `m4a`, `ogg`, `oga`, `wav`, `flac`, `opus`) |
| `[the log](run-17.log)` or `[[run-17.log]]` | A link that opens or downloads the file |

Pictures, video and audio are loaded only from Gantry itself. A picture address on another site is shown as a link.

## Blocks that draw

| Block | Content | Shown as |
| --- | --- | --- |
| ` ```mermaid ` | Mermaid text | A diagram, drawn in the page |
| ` ```diagram ` | Data written by the diagram editor | A drawn diagram. Edit it with **Draw a diagram**. |
| ` ```math ` | LaTeX | A formula |
| ` ```viz chartjs ` (or `echarts`, `plotly`, `d3`) | A script | A chart in a sealed frame. See [the visualization reference](visualizations.md). |
| ` ```embed ` | HTML, CSS, SVG, scripts | Your content in a sealed frame |

`viz` and `embed` accept `height=` and a number of pixels after the name, between 80 and 1200. Without it a chart is 360 pixels high and embedded HTML takes the height of its content.

## HTML

Written straight into the text, these elements are kept: headings, paragraphs and line breaks, `strong`, `b`, `em`, `i`, `u`, `s`, `del`, `ins`, `mark`, `small`, `sub`, `sup`, `kbd`, `abbr`, `code`, `pre`, `blockquote`, lists and definition lists, `a`, `img`, `video`, `audio`, `source`, tables, `span`, `div`, `details`, `summary`, `figure`, `figcaption`, `time`, `section`.

Everything else is removed before the text is shown, including `script`, `style`, `iframe`, `object`, `form` and its controls, event attributes such as `onclick`, inline `style` attributes, and addresses that begin with `javascript:`. Class names are kept only when they are ones this page describes, so text cannot dress itself up as part of the application.

To use styles, scripts, SVG with effects or anything else outside this list, put it in an ` ```embed ` block. It then runs in a sealed frame that cannot read or change the page around it.

## The simple editor

The simple page editor keeps a smaller set: headings, bold, italic, strikethrough, inline code, links, lists, task lists, quotes, tables, code blocks, pictures and horizontal rules. A page that uses anything else on this page opens in the advanced editor, so nothing is lost.
