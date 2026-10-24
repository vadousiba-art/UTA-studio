import { useCallback, useRef } from "react";
import { api } from "../../api/client";
import type { EditOp, StudioState } from "../../api/types";
import { useApp } from "../../state/app";

/** Studio actions with error handling and throttled transient edits. */
export function useStudioActions() {
  const setStudio = useApp((s) => s.setStudio);
  const error = useApp((s) => s.error);
  const lastTransient = useRef(0);
  const pending = useRef<EditOp | null>(null);

  const apply = useCallback(
    async (p: Promise<StudioState | null>) => {
      try {
        const st = await p;
        if (st) setStudio(st);
        return st;
      } catch (e) {
        error(e);
        return null;
      }
    },
    [setStudio, error],
  );

  /** Commit an edit (one undo step). */
  const edit = useCallback((op: EditOp) => apply(api.studioEdit(op, false)), [apply]);

  /**
   * Live edit while dragging: throttled to ~30 Hz, no undo step. Follow with
   * `edit(op)` on release to commit.
   */
  const live = useCallback(
    (op: EditOp) => {
      const now = performance.now();
      pending.current = op;
      if (now - lastTransient.current < 33) return;
      lastTransient.current = now;
      const o = pending.current;
      pending.current = null;
      api.studioEdit(o, true).catch(error);
    },
    [error],
  );

  const change = useCallback((op: EditOp, final: boolean) => (final ? void edit(op) : live(op)), [edit, live]);

  return {
    apply,
    edit,
    change,
    undo: () => apply(api.studioUndo()),
    redo: () => apply(api.studioRedo()),
    save: () => apply(api.studioSave()),
    refresh: () => apply(api.studioState()),
  };
}

export const framesToSec = (f: number, sr: number) => f / sr;
export const secToFrames = (s: number, sr: number) => Math.max(0, Math.round(s * sr));
