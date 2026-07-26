import { createContext, useContext } from "react";

/// Whether the connected instance is booted for inspection only, so every mutating endpoint refuses
/// with a `409`. The operator surfaces still render — the composer, the `self` editor, the retract
/// and merge affordances — but each holds its action closed with a note saying why, rather than
/// offering a control that can only fail.
///
/// A context rather than a prop because the fact is one the frame knows and only the leaves act on:
/// the views between them (the workspace, the state browser, the memory list) neither read it nor
/// have anything to say about it. The default is `false`, which is what the eval frame wants — a
/// finished run's views are already actionless, so it provides nothing and inherits the default.
export const ReadOnly = createContext(false);

export function useReadOnly(): boolean {
  return useContext(ReadOnly);
}
