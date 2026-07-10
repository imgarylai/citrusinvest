// A small CodeMirror 6 editor for the lemon DSL whose syntax highlighting is
// driven by the ENGINE, not a hand-written grammar. The Rust lexer (via
// `lemon-wasm.tokens()`) is the single source of truth for tokenization; this
// editor just paints the ranges it returns. That means highlighting can never
// drift from the language, and the same `tokens()` output can back other
// surfaces (citrus-fund, an LSP semantic-tokens provider) unchanged.

import { EditorState, RangeSetBuilder, StateEffect } from '@codemirror/state';
import {
  Decoration,
  EditorView,
  ViewPlugin,
  keymap,
  lineNumbers,
  placeholder,
} from '@codemirror/view';
import type { DecorationSet, ViewUpdate } from '@codemirror/view';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';

/** One classified span from `lemon-wasm.tokens()` (1-based, `endCol` exclusive). */
export interface HlToken {
  line: number;
  col: number;
  endLine: number;
  endCol: number;
  type: string;
}

/** Turns lemon source into classified tokens. Supplied once the WASM has loaded. */
export type Tokenizer = (src: string) => HlToken[];

// Effect used to force the highlighter to recompute when the tokenizer arrives
// (the WASM loads after the editor mounts).
const setHl = StateEffect.define<null>();

const MARK: Record<string, Decoration> = Object.fromEntries(
  [
    'comment',
    'number',
    'string',
    'keyword',
    'function',
    'parameter',
    'series',
    'operator',
    'punctuation',
  ].map((t) => [t, Decoration.mark({ class: `lm-${t}` })]),
);

const highlightTheme = EditorView.theme({
  '.lm-comment': { color: '#7c8494', fontStyle: 'italic' },
  '.lm-number': { color: '#d19a66' },
  '.lm-string': { color: '#c3e88d' },
  '.lm-keyword': { color: '#c678dd' },
  '.lm-function': { color: '#61afef' },
  '.lm-parameter': { color: '#e5c07b' },
  '.lm-series': { color: '#98c379' },
  '.lm-operator': { color: '#56b6c2' },
});

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
  },
  { dark: true },
);

export interface LemonEditor {
  getValue(): string;
  setValue(text: string): void;
  focus(): void;
  /** Wire up engine-driven highlighting once `lemon-wasm.tokens` is available. */
  setTokenizer(fn: Tokenizer): void;
}

export function createLemonEditor(
  parent: HTMLElement,
  doc: string,
  onRun: () => void,
): LemonEditor {
  // Mutable holder shared with the highlight plugin; null until the WASM loads.
  let tokenize: Tokenizer | null = null;

  const highlighter = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = this.build(view);
      }
      update(u: ViewUpdate) {
        const forced = u.transactions.some((tr) =>
          tr.effects.some((e) => e.is(setHl)),
        );
        if (u.docChanged || u.viewportChanged || forced) {
          this.decorations = this.build(u.view);
        }
      }
      build(view: EditorView): DecorationSet {
        const builder = new RangeSetBuilder<Decoration>();
        if (!tokenize) return builder.finish();
        const src = view.state.doc.toString();
        let toks: HlToken[];
        try {
          toks = tokenize(src);
        } catch {
          return builder.finish();
        }
        const docLen = view.state.doc.length;
        for (const t of toks) {
          const mark = MARK[t.type];
          if (!mark) continue;
          const from = view.state.doc.line(t.line).from + (t.col - 1);
          const to = view.state.doc.line(t.endLine).from + (t.endCol - 1);
          if (from >= 0 && to <= docLen && from < to) builder.add(from, to, mark);
        }
        return builder.finish();
      }
    },
    { decorations: (v) => v.decorations },
  );

  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc,
      extensions: [
        lineNumbers(),
        history(),
        placeholder('e.g. is_largest(sma(close, 2), 3)'),
        EditorView.lineWrapping,
        highlighter,
        highlightTheme,
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
    setTokenizer: (fn: Tokenizer) => {
      tokenize = fn;
      view.dispatch({ effects: setHl.of(null) }); // repaint with real tokens
    },
  };
}
