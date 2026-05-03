import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { api, type Settings, type SettingsPatch } from "../api/hindsight";

const SAVE_DEBOUNCE_MS = 250;

interface SettingsContextValue {
  settings: Settings | null;
  loading: boolean;
  update: (patch: SettingsPatch) => void;
}

const SettingsContext = createContext<SettingsContextValue | null>(null);

export function SettingsProvider({ children }: { children: ReactNode }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [loading, setLoading] = useState(true);
  const pendingRef = useRef<SettingsPatch>({});
  const timerRef = useRef<number | null>(null);

  useEffect(() => {
    api
      .getSettings()
      .then((s) => {
        setSettings(s);
      })
      .catch((e) => {
        console.error("加载设置失败:", e);
      })
      .finally(() => setLoading(false));
  }, []);

  const flush = useCallback(async () => {
    const patch = pendingRef.current;
    pendingRef.current = {};
    timerRef.current = null;
    if (Object.keys(patch).length === 0) return;
    try {
      const next = await api.updateSettings(patch);
      setSettings(next);
    } catch (e) {
      console.error("保存设置失败:", e);
    }
  }, []);

  const update = useCallback(
    (patch: SettingsPatch) => {
      setSettings((prev) => (prev ? { ...prev, ...patch } : prev));
      pendingRef.current = { ...pendingRef.current, ...patch };
      if (timerRef.current) window.clearTimeout(timerRef.current);
      timerRef.current = window.setTimeout(() => {
        void flush();
      }, SAVE_DEBOUNCE_MS);
    },
    [flush],
  );

  useEffect(() => {
    return () => {
      if (timerRef.current) {
        window.clearTimeout(timerRef.current);
        void flush();
      }
    };
  }, [flush]);

  const value = useMemo<SettingsContextValue>(
    () => ({ settings, loading, update }),
    [settings, loading, update],
  );

  return (
    <SettingsContext.Provider value={value}>{children}</SettingsContext.Provider>
  );
}

export function useSettings() {
  const ctx = useContext(SettingsContext);
  if (!ctx)
    throw new Error("useSettings must be used within SettingsProvider");
  return ctx;
}
