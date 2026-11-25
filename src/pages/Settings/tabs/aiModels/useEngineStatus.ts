import { useCallback, useEffect, useState } from "react";
import { api, type EngineStatus } from "../../../../api/hindsight";
import { logError } from "../../../../lib/logger";

export interface EngineStatusState {
  /** null until the first read comes back. */
  status: EngineStatus | null;
  refresh: () => Promise<void>;
}

/**
 * Read the engine and the text recognition component's status, for every section of the page.
 *
 * While the engine runs, it reads again every 5 seconds, so the countdown to freeing memory moves
 * and the state changes back once the engine stops by itself.
 */
export function useEngineStatus(): EngineStatusState {
  const [status, setStatus] = useState<EngineStatus | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStatus(await api.getEngineStatus());
    } catch (e) {
      logError("engine.getStatus", e);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const running = status?.runtime.state === "running";
  useEffect(() => {
    if (!running) return;
    const id = window.setInterval(() => void refresh(), 5_000);
    return () => window.clearInterval(id);
  }, [running, refresh]);

  return { status, refresh };
}
