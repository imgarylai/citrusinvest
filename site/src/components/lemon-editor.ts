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
  hoverTooltip,
  keymap,
  lineNumbers,
  placeholder,
} from '@codemirror/view';
import type { DecorationSet, Tooltip, ViewUpdate } from '@codemirror/view';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import {
  autocompletion,
  completionKeymap,
  type Completion,
  type CompletionContext,
  type CompletionResult,
} from '@codemirror/autocomplete';
import { forceLinting, linter, lintKeymap, type Diagnostic } from '@codemirror/lint';

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

/** Hover payload from `lemon-wasm.hover()` (1-based range, `endCol` exclusive). */
export interface HoverInfo {
  line: number;
  col: number;
  endLine: number;
  endCol: number;
  markdown: string;
}

/** One candidate from `lemon-wasm.completions()`. */
export interface LemonCompletion {
  label: string;
  kind: string;
  detail: string;
  documentation: string;
  insertText: string;
}

/** One warning from `lemon-wasm.lint()` (1-based `col`). */
export interface LemonLint {
  line: number;
  col: number;
  message: string;
}

/**
 * The engine-backed language services, all null until the WASM loads. Every one
 * takes/returns the same shapes the `lemon-wasm` JSON boundary uses, already
 * parsed from JSON by the caller.
 */
export interface LemonServices {
  hover(src: string, line: number, col: number): HoverInfo | null;
  completions(src: string, line: number, col: number): LemonCompletion[];
  /** Semantic warnings; `null` when the source failed to parse (a parse error
   *  is surfaced by the run button, not the linter). */
  lint(src: string): LemonLint[] | null;
}

// Effect used to force the highlighter to recompute when the tokenizer arrives
// (the WASM loads after the editor mounts).
const setHl = StateEffect.define<null>();

/** CodeMirror completion `type` for each engine `CompletionKind`. */
const COMPLETION_TYPE: Record<string, string> = {
  function: 'function',
  field: 'property',
  variable: 'variable',
  series: 'class',
  keyword: 'keyword',
};

/**
 * Render the small, trusted markdown the engine returns for hovers into a DOM
 * node: fenced ```lemon blocks, inline `code`, **bold**, *italic*, and blank-line
 * paragraphs. The source is engine-generated (op catalog / keyword table), not
 * user input, but we still build nodes via textContent so no markup is injected.
 */
function renderHoverMarkdown(md: string): HTMLElement {
  const root = document.createElement('div');
  root.className = 'lm-hover';
  // Split into fenced code blocks vs. prose. Fences look like ```lemon\n…\n```.
  const parts = md.split(/```(?:lemon)?\n?/);
  parts.forEach((part, i) => {
    if (!part) return;
    if (i % 2 === 1) {
      const pre = document.createElement('pre');
      pre.className = 'lm-hover-code';
      pre.textContent = part.replace(/\n$/, '');
      root.appendChild(pre);
    } else {
      for (const para of part.split(/\n{2,}/)) {
        const trimmed = para.trim();
        if (!trimmed) continue;
        const p = document.createElement('p');
        appendInline(p, trimmed);
        root.appendChild(p);
      }
    }
  });
  return root;
}

/** Append inline markdown (`code`, **bold**, *italic*) to `el` as text/element nodes. */
function appendInline(el: HTMLElement, text: string): void {
  // Tokenize on the three inline markers; each capturing group keeps its markers.
  const re = /(`[^`]+`|\*\*[^*]+\*\*|\*[^*]+\*)/g;
  let last = 0;
  for (const m of text.matchAll(re)) {
    const idx = m.index ?? 0;
    if (idx > last) el.appendChild(document.createTextNode(text.slice(last, idx)));
    const tok = m[0];
    if (tok.startsWith('`')) {
      const code = document.createElement('code');
      code.textContent = tok.slice(1, -1);
      el.appendChild(code);
    } else if (tok.startsWith('**')) {
      const b = document.createElement('strong');
      b.textContent = tok.slice(2, -2);
      el.appendChild(b);
    } else {
      const em = document.createElement('em');
      em.textContent = tok.slice(1, -1);
      el.appendChild(em);
    }
    last = idx + tok.length;
  }
  if (last < text.length) el.appendChild(document.createTextNode(text.slice(last)));
}

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
    // Hover tooltip + autocomplete popup: match the terminal-report look.
    '.cm-tooltip': {
      border: '1px solid var(--sl-color-gray-5)',
      borderRadius: '0.4rem',
      background: 'var(--sl-color-black)',
      color: 'var(--sl-color-white)',
    },
    '.cm-tooltip.cm-tooltip-autocomplete > ul': {
      fontFamily: 'var(--sl-font-mono, ui-monospace, monospace)',
      fontSize: '0.82rem',
    },
    '.cm-tooltip-autocomplete ul li[aria-selected]': {
      background: 'var(--sl-color-accent-low)',
      color: 'var(--sl-color-white)',
    },
    '.cm-completionIcon': { opacity: '0.7' },
    '.cm-completionDetail': { color: 'var(--sl-color-gray-3)', fontStyle: 'normal' },
    '.lm-hover': {
      fontFamily: 'var(--sl-font-sans, system-ui, sans-serif)',
      fontSize: '0.82rem',
      lineHeight: '1.5',
      maxWidth: '32rem',
      padding: '0.5rem 0.7rem',
    },
    '.lm-hover p': { margin: '0 0 0.4rem' },
    '.lm-hover p:last-child': { margin: '0' },
    '.lm-hover code': {
      fontFamily: 'var(--sl-font-mono, ui-monospace, monospace)',
      fontSize: '0.9em',
    },
    '.lm-hover-code': {
      fontFamily: 'var(--sl-font-mono, ui-monospace, monospace)',
      fontSize: '0.82rem',
      background: 'var(--sl-color-gray-6, rgba(255,255,255,0.06))',
      borderRadius: '0.3rem',
      padding: '0.4rem 0.55rem',
      margin: '0 0 0.5rem',
      whiteSpace: 'pre-wrap',
      color: 'var(--sl-color-accent-high)',
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
  /** Wire up hover / completion / lint once the `lemon-wasm` services load. */
  setServices(services: LemonServices): void;
}

/** 1-based (line, col) → absolute document offset, clamped to the line. */
function offsetOf(view: EditorView, line: number, col: number): number {
  const doc = view.state.doc;
  if (line < 1 || line > doc.lines) return -1;
  const l = doc.line(line);
  return Math.min(l.from + Math.max(0, col - 1), l.to);
}

/** Absolute offset → 1-based (line, col). */
function lineColOf(view: EditorView, pos: number): { line: number; col: number } {
  const l = view.state.doc.lineAt(pos);
  return { line: l.number, col: pos - l.from + 1 };
}

export function createLemonEditor(
  parent: HTMLElement,
  doc: string,
  onRun: () => void,
  onChange?: (value: string) => void,
): LemonEditor {
  // Mutable holders shared with the plugins; null until the WASM loads.
  let tokenize: Tokenizer | null = null;
  let services: LemonServices | null = null;

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

  // Hover: markdown docs for the token under the pointer, from the engine.
  const hoverExt = hoverTooltip((view, pos): Tooltip | null => {
    if (!services) return null;
    const { line, col } = lineColOf(view, pos);
    let info: HoverInfo | null;
    try {
      info = services.hover(view.state.doc.toString(), line, col);
    } catch {
      return null;
    }
    if (!info) return null;
    const from = offsetOf(view, info.line, info.col);
    const end = offsetOf(view, info.endLine, info.endCol);
    if (from < 0) return null;
    return {
      pos: from,
      end: end > from ? end : undefined,
      above: true,
      create: () => ({ dom: renderHoverMarkdown(info.markdown) }),
    };
  });

  // Completion: the engine's context-aware candidate list.
  const completionSource = (context: CompletionContext): CompletionResult | null => {
    if (!services || !context.view) return null;
    const word = context.matchBefore(/[A-Za-z_][A-Za-z0-9_]*/);
    // Only auto-open on a word; still allow explicit (Ctrl-Space) invocation.
    if (!word && !context.explicit) return null;
    const { line, col } = lineColOf(context.view, context.pos);
    let items: LemonCompletion[];
    try {
      items = services.completions(context.state.doc.toString(), line, col);
    } catch {
      return null;
    }
    if (items.length === 0) return null;
    const options: Completion[] = items.map((c) => ({
      label: c.label,
      type: COMPLETION_TYPE[c.kind] ?? 'text',
      detail: c.detail || undefined,
      info: c.documentation || undefined,
      apply: c.insertText,
    }));
    return { from: word ? word.from : context.pos, options, validFor: /^[A-Za-z0-9_]*$/ };
  };

  // Lint: engine semantic warnings (unknown series, unused `let`s). Parse errors
  // return null and are surfaced by the run button, so the linter stays quiet.
  const lintExt = linter(
    (view): Diagnostic[] => {
      if (!services) return [];
      let lints: LemonLint[] | null;
      try {
        lints = services.lint(view.state.doc.toString());
      } catch {
        return [];
      }
      if (!lints) return [];
      const out: Diagnostic[] = [];
      for (const l of lints) {
        const from = offsetOf(view, l.line, l.col);
        if (from < 0) continue;
        // Span the word at the position so the squiggle covers the whole name.
        const lineText = view.state.doc.lineAt(from).text;
        const rel = from - view.state.doc.lineAt(from).from;
        const m = /^[A-Za-z0-9_]+/.exec(lineText.slice(rel));
        out.push({
          from,
          to: m ? from + m[0].length : Math.min(from + 1, view.state.doc.length),
          severity: 'warning',
          message: l.message,
        });
      }
      return out;
    },
    { delay: 300 },
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
        // Notify on every document change so callers can mirror the live source
        // (e.g. the "Run this locally" .lemon preview under the landing widget).
        ...(onChange
          ? [
              EditorView.updateListener.of((u) => {
                if (u.docChanged) onChange(u.state.doc.toString());
              }),
            ]
          : []),
        hoverExt,
        autocompletion({ override: [completionSource] }),
        lintExt,
        keymap.of([
          {
            key: 'Mod-Enter',
            run: () => {
              onRun();
              return true;
            },
          },
          ...completionKeymap,
          ...lintKeymap,
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
    setServices: (fn: LemonServices) => {
      services = fn;
      forceLinting(view); // re-run the linter now that the engine is available
    },
  };
}
