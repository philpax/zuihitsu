import CodeMirror, { EditorView } from "@uiw/react-codemirror";
import { StreamLanguage } from "@codemirror/language";
import { lua } from "@codemirror/legacy-modes/mode/lua";

const luaLanguage = StreamLanguage.define(lua);

/// A calm light theme over the design tokens — paper ground, sumi-ink text, a clay caret — so the
/// editor sits inside the Japandi system rather than shipping its own chrome.
const theme = EditorView.theme({
  "&": { backgroundColor: "transparent", color: "var(--color-ink)" },
  ".cm-content": {
    fontFamily: "var(--font-mono)",
    fontSize: "0.8125rem",
    caretColor: "var(--color-clay)",
    padding: "0.5rem 0",
  },
  ".cm-gutters": {
    backgroundColor: "transparent",
    border: "none",
    color: "color-mix(in oklab, var(--color-ink-faint) 70%, transparent)",
  },
  ".cm-cursor": { borderLeftColor: "var(--color-clay)" },
  "&.cm-focused": { outline: "none" },
  ".cm-activeLine, .cm-activeLineGutter": { backgroundColor: "transparent" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": {
    backgroundColor: "var(--color-oat-deep)",
  },
  ".cm-line": { padding: "0 0.5rem" },
});

/// A real Lua code editor (CodeMirror 6) for the operator console. `onSubmit` fires on Cmd/Ctrl+Enter
/// so a block runs without leaving the keyboard.
export function CodeEditor({
  value,
  onChange,
  onSubmit,
  disabled = false,
}: {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  disabled?: boolean;
}) {
  return (
    <div
      className="rounded-xs border border-line bg-paper-raised"
      onKeyDownCapture={(event) => {
        if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
          event.preventDefault();
          onSubmit();
        }
      }}
    >
      <CodeMirror
        value={value}
        onChange={onChange}
        theme={theme}
        extensions={[luaLanguage]}
        editable={!disabled}
        basicSetup={{
          lineNumbers: true,
          foldGutter: false,
          highlightActiveLine: false,
          highlightActiveLineGutter: false,
          autocompletion: false,
          searchKeymap: false,
        }}
        minHeight="6rem"
      />
    </div>
  );
}
