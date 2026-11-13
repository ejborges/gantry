// A plain code editor for visualization scripts and custom HTML: JavaScript or HTML
// highlighting, and, when several people have the same script open, everyone's cursor.
import { useEffect, useRef } from "react";
import { EditorState } from "@codemirror/state";
import { EditorView, keymap, lineNumbers, highlightActiveLine } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { HighlightStyle, bracketMatching, indentOnInput, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import { closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
import { javascript } from "@codemirror/lang-javascript";
import { html } from "@codemirror/lang-html";
import { yCollab, yUndoManagerKeymap } from "y-codemirror.next";
import type { SharedDoc } from "../realtime";

// Colors come from the application's tokens, so the code reads well in both themes.
const highlight = HighlightStyle.define([
  { tag: [t.keyword, t.controlKeyword, t.operatorKeyword, t.definitionKeyword, t.moduleKeyword], color: "var(--purple)" },
  { tag: [t.string, t.special(t.string), t.regexp, t.attributeValue], color: "var(--done)" },
  { tag: [t.number, t.bool, t.null, t.atom], color: "var(--warn)" },
  { tag: [t.comment, t.lineComment, t.blockComment], color: "var(--muted)", fontStyle: "italic" },
  { tag: [t.function(t.variableName), t.function(t.propertyName)], color: "var(--action)" },
  { tag: [t.propertyName, t.attributeName], color: "var(--teal)" },
  { tag: [t.tagName, t.typeName, t.className], color: "var(--danger)" },
  { tag: [t.punctuation, t.bracket, t.operator], color: "var(--ink-2)" },
  { tag: t.invalid, color: "var(--danger)", textDecoration: "underline wavy" },
]);

interface Props {
  value: string;
  onChange: (text: string) => void;
  language: "javascript" | "html";
  label: string;
  readOnly?: boolean;
  shared?: SharedDoc | null;
  /** Ctrl+Enter or ⌘+Enter. */
  onRun?: () => void;
}

export default function CodeEditor({ value, onChange, language, label, readOnly, shared, onRun }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const latest = useRef({ onChange, onRun });
  latest.current = { onChange, onRun };

  useEffect(() => {
    if (!host.current) return;
    const ytext = shared?.ydoc.getText("t");
    const v = new EditorView({
      parent: host.current,
      state: EditorState.create({
        doc: ytext ? ytext.toString() : value,
        extensions: [
          lineNumbers(),
          highlightActiveLine(),
          indentOnInput(),
          bracketMatching(),
          closeBrackets(),
          ytext ? [yCollab(ytext, shared!.awareness), keymap.of(yUndoManagerKeymap)] : [history(), keymap.of(historyKeymap)],
          keymap.of([{ key: "Mod-Enter", run: () => (latest.current.onRun?.(), true) }, ...closeBracketsKeymap, ...defaultKeymap, indentWithTab]),
          language === "html" ? html() : javascript(),
          syntaxHighlighting(highlight),
          EditorState.readOnly.of(!!readOnly || (!!shared && !shared.canWrite)),
          EditorState.tabSize.of(2),
          // Tab indents here, so say how to leave the editor with the keyboard.
          // A read-only script can still be scrolled and read with the keyboard.
          EditorView.contentAttributes.of({ "aria-label": readOnly ? label : `${label}. Press Escape, then Tab, to leave the editor.`, "aria-multiline": "true", "aria-readonly": readOnly ? "true" : "false", role: "textbox", tabindex: "0", spellcheck: "false", autocapitalize: "off" }),
          EditorView.updateListener.of((u) => u.docChanged && latest.current.onChange(u.state.doc.toString())),
        ],
      }),
    });
    view.current = v;
    if (ytext) latest.current.onChange(ytext.toString());
    return () => {
      v.destroy();
      view.current = null;
    };
    // A different shared document, language or mode means a new editor.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shared, language, readOnly]);

  // Text replaced from outside (a copy loaded, a save by another route) while not sharing.
  useEffect(() => {
    const v = view.current;
    if (v && !shared && v.state.doc.toString() !== value) v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: value } });
  }, [value, shared]);

  return <div ref={host} className="code-editor" />;
}
