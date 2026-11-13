// Simple editor: what you see is what you get. It edits rich text and stores Markdown,
// so a page written here opens unchanged in the advanced editor, the API and MCP tools.
import { useEffect } from "react";
import { EditorContent, useEditor, type Editor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import LinkExt from "@tiptap/extension-link";
import Placeholder from "@tiptap/extension-placeholder";
import Table from "@tiptap/extension-table";
import TableRow from "@tiptap/extension-table-row";
import TableCell from "@tiptap/extension-table-cell";
import TableHeader from "@tiptap/extension-table-header";
import TaskList from "@tiptap/extension-task-list";
import TaskItem from "@tiptap/extension-task-item";
import { Bold, Code, Heading1, Heading2, Heading3, Italic, Link, List, ListOrdered, ListTodo, Minus, Quote, Redo2, SquareCode, Strikethrough, Table as TableIcon, Undo2, Unlink } from "lucide-react";
import Collaboration from "@tiptap/extension-collaboration";
import CollaborationCursor from "@tiptap/extension-collaboration-cursor";
import { generateJSON, getSchema, type Extensions } from "@tiptap/core";
import { prosemirrorJSONToYXmlFragment } from "y-prosemirror";
import type * as Y from "yjs";
import { renderForEditor, toMarkdown } from "../lib/markdown";
import { colorFor, type SharedDoc } from "../realtime";
import type { Person } from "../api";

/** What the simple editor can represent. Editing together replaces its own undo history. */
function extensions(placeholder: string | undefined, together: boolean): Extensions {
  return [
    StarterKit.configure({ heading: { levels: [1, 2, 3] }, ...(together ? { history: false } : {}) }),
    LinkExt.configure({ openOnClick: false, autolink: true, protocols: ["http", "https", "mailto"], HTMLAttributes: { rel: "noopener noreferrer" } }),
    Placeholder.configure({ placeholder: placeholder ?? "Start writing…" }),
    Table.configure({ resizable: false }),
    TableRow,
    TableHeader,
    TableCell,
    TaskList,
    TaskItem.configure({ nested: true }),
  ];
}

/** Load saved Markdown into an empty shared document, in the form the simple editor edits. */
export function seedRich(doc: Y.Doc, markdown: string) {
  const ext = extensions(undefined, true);
  prosemirrorJSONToYXmlFragment(getSchema(ext), generateJSON(renderForEditor(markdown), ext), doc.getXmlFragment("default"));
}

function Tool({ editor, label, active, onClick, children, disabled }: { editor: Editor; label: string; active?: boolean; onClick: () => void; children: React.ReactNode; disabled?: boolean }) {
  return (
    <button type="button" className={`btn ghost icon sm ${active ? "on" : ""}`} title={label} aria-label={label} aria-pressed={active} disabled={disabled || !editor.isEditable} onMouseDown={(e) => e.preventDefault()} onClick={onClick}>
      {children}
    </button>
  );
}

export default function SimpleEditor({ value, onChange, placeholder, shared, me }: { value: string; onChange: (markdown: string) => void; placeholder?: string; shared?: SharedDoc | null; me?: Person }) {
  const editor = useEditor(
    {
      extensions: [
        ...extensions(placeholder, !!shared),
        ...(shared ? [Collaboration.configure({ document: shared.ydoc }), CollaborationCursor.configure({ provider: { awareness: shared.awareness }, user: { name: me?.name ?? "Someone", color: colorFor(me?.id ?? "") } })] : []),
      ],
      // Editing together, the content is the shared document's; alone, it is the saved text.
      content: shared ? undefined : renderForEditor(value),
      editable: !shared || shared.canWrite,
      editorProps: { attributes: { class: "prose", "aria-label": "Page content", role: "textbox", "aria-multiline": "true" } },
      onUpdate: ({ editor }) => onChange(editor.isEmpty ? "" : toMarkdown(editor.getHTML())),
    },
    [shared],
  );
  // Load new content when a template is applied or the page is switched from outside.
  useEffect(() => {
    if (editor && !shared && !editor.isFocused && toMarkdown(editor.getHTML()).trim() !== value.trim()) editor.commands.setContent(renderForEditor(value), false);
  }, [editor, value, shared]);
  // What the others already typed is this editor's starting text.
  useEffect(() => {
    if (editor && shared) onChange(editor.isEmpty ? "" : toMarkdown(editor.getHTML()));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editor, shared]);
  if (!editor) return null;
  const c = () => editor.chain().focus();
  const setLink = () => {
    const prev = editor.getAttributes("link").href as string | undefined;
    const url = window.prompt("Link address (https://…)", prev ?? "https://");
    if (url === null) return;
    if (url.trim() === "" || !/^(https?:\/\/|mailto:|\/)/i.test(url.trim())) c().unsetLink().run();
    else c().extendMarkRange("link").setLink({ href: url.trim() }).run();
  };
  return (
    <div className="editor">
      <div className="toolbar" role="toolbar" aria-label="Formatting">
        <Tool editor={editor} label="Heading 1" active={editor.isActive("heading", { level: 1 })} onClick={() => c().toggleHeading({ level: 1 }).run()}><Heading1 /></Tool>
        <Tool editor={editor} label="Heading 2" active={editor.isActive("heading", { level: 2 })} onClick={() => c().toggleHeading({ level: 2 }).run()}><Heading2 /></Tool>
        <Tool editor={editor} label="Heading 3" active={editor.isActive("heading", { level: 3 })} onClick={() => c().toggleHeading({ level: 3 }).run()}><Heading3 /></Tool>
        <span className="sep" />
        <Tool editor={editor} label="Bold" active={editor.isActive("bold")} onClick={() => c().toggleBold().run()}><Bold /></Tool>
        <Tool editor={editor} label="Italic" active={editor.isActive("italic")} onClick={() => c().toggleItalic().run()}><Italic /></Tool>
        <Tool editor={editor} label="Strikethrough" active={editor.isActive("strike")} onClick={() => c().toggleStrike().run()}><Strikethrough /></Tool>
        <Tool editor={editor} label="Inline code" active={editor.isActive("code")} onClick={() => c().toggleCode().run()}><Code /></Tool>
        <span className="sep" />
        <Tool editor={editor} label="Bulleted list" active={editor.isActive("bulletList")} onClick={() => c().toggleBulletList().run()}><List /></Tool>
        <Tool editor={editor} label="Numbered list" active={editor.isActive("orderedList")} onClick={() => c().toggleOrderedList().run()}><ListOrdered /></Tool>
        <Tool editor={editor} label="Checklist" active={editor.isActive("taskList")} onClick={() => c().toggleTaskList().run()}><ListTodo /></Tool>
        <Tool editor={editor} label="Quote" active={editor.isActive("blockquote")} onClick={() => c().toggleBlockquote().run()}><Quote /></Tool>
        <Tool editor={editor} label="Code block" active={editor.isActive("codeBlock")} onClick={() => c().toggleCodeBlock().run()}><SquareCode /></Tool>
        <span className="sep" />
        <Tool editor={editor} label="Link" active={editor.isActive("link")} onClick={setLink}><Link /></Tool>
        {editor.isActive("link") && <Tool editor={editor} label="Remove link" onClick={() => c().unsetLink().run()}><Unlink /></Tool>}
        <Tool editor={editor} label="Insert table" onClick={() => c().insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run()}><TableIcon /></Tool>
        <Tool editor={editor} label="Divider" onClick={() => c().setHorizontalRule().run()}><Minus /></Tool>
        {editor.isActive("table") && (
          <>
            <span className="sep" />
            <button type="button" className="btn ghost sm" onMouseDown={(e) => e.preventDefault()} onClick={() => c().addRowAfter().run()}>Add row</button>
            <button type="button" className="btn ghost sm" onMouseDown={(e) => e.preventDefault()} onClick={() => c().addColumnAfter().run()}>Add column</button>
            <button type="button" className="btn ghost sm" onMouseDown={(e) => e.preventDefault()} onClick={() => c().deleteRow().run()}>Delete row</button>
            <button type="button" className="btn ghost sm" onMouseDown={(e) => e.preventDefault()} onClick={() => c().deleteColumn().run()}>Delete column</button>
          </>
        )}
        <span className="grow" />
        <Tool editor={editor} label="Undo" disabled={!editor.can().undo()} onClick={() => c().undo().run()}><Undo2 /></Tool>
        <Tool editor={editor} label="Redo" disabled={!editor.can().redo()} onClick={() => c().redo().run()}><Redo2 /></Tool>
      </div>
      <EditorContent editor={editor} />
    </div>
  );
}
