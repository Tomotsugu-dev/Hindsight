import { useCallback, useEffect, useState } from "react";
import { api, type CaptureStatus } from "../api/hindsight";

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
    refresh();
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
      console.error("切换采集状态失败:", e);
    }
  }, [status, refresh]);

  return { status, toggle, refresh };
}
