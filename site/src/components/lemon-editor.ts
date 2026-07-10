// A small CodeMirror 6 editor with syntax highlighting for the lemon DSL.
//
// The tokenizer is intentionally structural rather than a hard-coded op list, so
// it never drifts from the language: an identifier before `(` is a function
// call, an identifier before `=` is a keyword argument, `and`/`or`/`not` are
// logic keywords, everything else bareword is a series reference. That colours
// `is_largest(sma(close, 2), 3)` correctly without knowing those op names.

import { EditorState } from '@codemirror/state';
import { EditorView, keymap, lineNumbers, placeholder } from '@codemirror/view';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import {
  HighlightStyle,
  StreamLanguage,
  syntaxHighlighting,
} from '@codemirror/language';
import { tags as t } from '@lezer/highlight';

const lemonLanguage = StreamLanguage.define<Record<string, never>>({
  startState: () => ({}),
  token(stream) {
    if (stream.eatSpace()) return null;
    if (stream.match(/#.*/)) return 'comment';
    if (stream.match(/[0-9]+(\.[0-9]+)?/)) return 'number';
    if (stream.match(/\b(?:and|or|not)\b/)) return 'logic';
    if (stream.match(/[A-Za-z_][A-Za-z0-9_]*/)) {
      const rest = stream.string.slice(stream.pos);
      if (/^\s*\(/.test(rest)) return 'fn'; // call: name(
      if (/^\s*=/.test(rest)) return 'kwarg'; // keyword arg: name=
      return 'series'; // bareword series (close, pe, …)
    }
    if (stream.match(/>=|<=|[><+\-*/=]/)) return 'op';
    stream.next();
    return null;
  },
  tokenTable: {
    comment: t.lineComment,
    number: t.number,
    logic: t.keyword,
    fn: t.function(t.variableName),
    kwarg: t.propertyName,
    series: t.variableName,
    op: t.operator,
  },
});

// Colours read well on the dark editor surface used by the playground.
const lemonHighlight = HighlightStyle.define([
  { tag: t.lineComment, color: '#7c8494', fontStyle: 'italic' },
  { tag: t.number, color: '#d19a66' },
  { tag: t.keyword, color: '#c678dd' },
  { tag: t.function(t.variableName), color: '#61afef' },
  { tag: t.propertyName, color: '#e5c07b' },
  { tag: t.variableName, color: '#98c379' },
  { tag: t.operator, color: '#56b6c2' },
]);

const editorTheme = EditorView.theme(
  {
    '&': {
      fontSize: '0.9rem',
      border: '1px solid var(--sl-color-gray-5)',
      borderRadius: '0.5rem',
      background: 'var(--sl-color-black)',
      color: 'var(--sl-color-white)',
    },
    '&.cm-focused': { outline: '2px solid var(--sl-color-accent)' },
    '.cm-content': {
      fontFamily: 'var(--sl-font-mono, ui-monospace, monospace)',
      minHeight: '10rem',
    },
    '.cm-gutters': {
      background: 'transparent',
      color: 'var(--sl-color-gray-4)',
      border: 'none',
    },
    '.cm-scroller': { borderRadius: '0.5rem' },
  },
  { dark: true },
);

export interface LemonEditor {
  getValue(): string;
  setValue(text: string): void;
  focus(): void;
}

export function createLemonEditor(
  parent: HTMLElement,
  doc: string,
  onRun: () => void,
): LemonEditor {
  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc,
      extensions: [
        lineNumbers(),
        history(),
        placeholder('e.g. is_largest(sma(close, 2), 3)'),
        EditorView.lineWrapping,
        lemonLanguage,
        syntaxHighlighting(lemonHighlight),
        editorTheme,
        keymap.of([
          {
            key: 'Mod-Enter',
            run: () => {
              onRun();
              return true;
            },
          },
          ...defaultKeymap,
          ...historyKeymap,
        ]),
      ],
    }),
  });

  return {
    getValue: () => view.state.doc.toString(),
    setValue: (text: string) =>
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: text },
      }),
    focus: () => view.focus(),
  };
}
