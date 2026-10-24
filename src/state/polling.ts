import { useEffect, useRef } from "react";
import { api, isTauri } from "../api/client";
import type { LiveFrame } from "../api/types";
import { useApp } from "./app";

/**
 * Polls engine status (meters, transport) at ~25 Hz while the window is
 * visible, and capture status at ~4 Hz. Polling stops when hidden to save CPU.
 */
export function useEnginePolling() {
  const setStatus = useApp((s) => s.setStatus);
  const setCapture = useApp((s) => s.setCapture);
  useEffect(() => {
    if (!isTauri()) return;
    let alive = true;
    let busy = false;
    let tick = 0;
    let lostNotified = false;
    const id = window.setInterval(async () => {
      if (!alive || busy || document.visibilityState === "hidden") return;
      busy = true;
      try {
        const st = await api.engineStatus();
        setStatus(st);
        if (st.deviceLost && !lostNotified) {
          lostNotified = true;
          useApp.getState().toast("The audio device was disconnected. Choose a device in Settings to reconnect.", "error");
        }
        if (!st.deviceLost) lostNotified = false;
        if (tick++ % 6 === 0) setCapture(await api.captureStatus());
      } catch {
        setStatus(null);
      } finally {
        busy = false;
      }
    }, 40);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [setStatus, setCapture]);
}

/** Live pitch frames while `enabled`; returns a ref holding recent frames. */
export function useLivePitch(enabled: boolean, keep = 600) {
  const frames = useRef<LiveFrame[]>([]);
  const since = useRef(0);
  useEffect(() => {
    if (!isTauri()) return;
    let alive = true;
    api.setLivePitch(enabled).catch(() => undefined);
    if (!enabled) {
      frames.current = [];
      return;
    }
    const id = window.setInterval(async () => {
      if (!alive || document.visibilityState === "hidden") return;
      try {
        const f = await api.livePitch(since.current);
        if (f.length) {
          since.current = f[f.length - 1]!.seq;
          const merged = frames.current.concat(f);
          frames.current = merged.length > keep ? merged.slice(merged.length - keep) : merged;
        }
      } catch {
        /* engine restarting; try again next tick */
      }
    }, 50);
    return () => {
      alive = false;
      window.clearInterval(id);
      api.setLivePitch(false).catch(() => undefined);
    };
  }, [enabled, keep]);
  return frames;
}
