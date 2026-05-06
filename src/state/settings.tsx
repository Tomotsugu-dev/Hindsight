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
import { logError } from "../lib/logger";

const SAVE_DEBOUNCE_MS = 250;

interface SettingsContextValue {
  settings: Settings | null;
  loading: boolean;
  update: (patch: SettingsPatch) => void;
  /** 后端 settings 被旁路命令（如 set_active_model）改写后，前端调一下重读，
   *  让 useSettings 订阅者拿到新值。普通改设置不要用这个，用 update。 */
  reload: () => Promise<void>;
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
        logError("settings.load", e);
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
      logError("settings.save", e);
    }
  }, []);

  const reload = useCallback(async () => {
    try {
      setSettings(await api.getSettings());
    } catch (e) {
      logError("settings.reload", e);
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
    () => ({ settings, loading, update, reload }),
    [settings, loading, update, reload],
  );

  return (
    <SettingsContext.Provider value={value}>{children}</SettingsContext.Provider>
  );
}

// hook 跟 Provider 同文件是有意为之——消费方一次 import 解决，dev 期 fast refresh
// 在改动 Provider 时退化为整页刷新（state 文件极少改动，影响可接受）。
// eslint-disable-next-line react-refresh/only-export-components
export function useSettings() {
  const ctx = useContext(SettingsContext);
  if (!ctx)
    throw new Error("useSettings must be used within SettingsProvider");
  return ctx;
}
