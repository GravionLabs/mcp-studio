import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  OnDestroy,
  afterNextRender,
  effect,
  inject,
  input,
  output,
  viewChild,
} from "@angular/core";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { json, jsonParseLinter } from "@codemirror/lang-json";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { linter, lintGutter } from "@codemirror/lint";
import { EditorState } from "@codemirror/state";
import { EditorView, keymap, lineNumbers } from "@codemirror/view";
import { tags } from "@lezer/highlight";

const highlight = HighlightStyle.define([
  { tag: tags.propertyName, color: "var(--accent)" },
  { tag: tags.string, color: "var(--ok)" },
  { tag: [tags.number, tags.bool, tags.null], color: "var(--warn)" },
]);

const theme = EditorView.theme({
  "&": {
    backgroundColor: "var(--bg)",
    color: "var(--text)",
    border: "1px solid var(--border)",
    borderRadius: "var(--radius)",
  },
  ".cm-content": { fontFamily: "var(--font-mono)", caretColor: "var(--text)" },
  ".cm-gutters": {
    backgroundColor: "var(--bg-elevated)",
    color: "var(--text-muted)",
    border: "none",
  },
  "&.cm-focused": { outline: "2px solid var(--accent)", outlineOffset: "-1px" },
  ".cm-scroller": { maxHeight: "320px", overflow: "auto" },
});

/** JSON text editor (CodeMirror 6) with syntax checking. Schema problems are shown by the parent. */
@Component({
  selector: "app-json-editor",
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<div #host class="host"></div>`,
  styles: `
    .host {
      min-width: 0;
    }
  `,
})
export class JsonEditorComponent implements OnDestroy {
  readonly value = input.required<string>();
  readonly valueChange = output<string>();

  private readonly host = viewChild.required<ElementRef<HTMLElement>>("host");
  private view: EditorView | null = null;
  private readonly element = inject<ElementRef<HTMLElement>>(ElementRef);

  constructor() {
    afterNextRender(() => {
      this.view = new EditorView({
        parent: this.host().nativeElement,
        state: EditorState.create({
          doc: this.value(),
          extensions: [
            lineNumbers(),
            history(),
            keymap.of([...defaultKeymap, ...historyKeymap]),
            json(),
            linter(jsonParseLinter()),
            lintGutter(),
            syntaxHighlighting(highlight),
            theme,
            EditorView.updateListener.of((update) => {
              if (update.docChanged) this.valueChange.emit(update.state.doc.toString());
            }),
          ],
        }),
      });
    });

    // Push external changes into the editor without clobbering what the user just typed.
    effect(() => {
      const next = this.value();
      const view = this.view;
      if (view && view.state.doc.toString() !== next) {
        view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: next } });
      }
    });
  }

  ngOnDestroy(): void {
    this.view?.destroy();
    this.view = null;
    void this.element;
  }
}
