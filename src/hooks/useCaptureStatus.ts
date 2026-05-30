import { useCallback, useEffect, useState } from "react";
import { api, type CaptureStatus } from "../api/hindsight";
import { logError } from "../lib/logger";

const POLL_MS = 5000;

export function useCaptureStatus() {
  const [status, setStatus] = useState<CaptureStatus | null>(null);

  const refresh = useCallback(async () => {
    try {
      const s = await api.getCaptureStatus();
      setStatus(s);
    } catch {
      /* ignore */
    }
  }, []);

  useEffect(() => {
    void refresh();
    const t = setInterval(refresh, POLL_MS);
    return () => clearInterval(t);
  }, [refresh]);

  const toggle = useCallback(async () => {
    if (!status) return;
    try {
      if (status.running) await api.stopCapture();
      else await api.startCapture();
      await refresh();
    } catch (e) {
      logError("capture.toggle", e);
    }
  }, [status, refresh]);

  return { status, toggle, refresh };
}
