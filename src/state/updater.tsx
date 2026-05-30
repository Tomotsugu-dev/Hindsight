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
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { useTranslation } from "react-i18next";
import { ConfirmDialog } from "../components/ConfirmDialog/ConfirmDialog";
import { useSettings } from "./settings";

export type UpdatePhase =
  | "idle"
  | "checking"
  | "uptodate"
  | "installing"
  | "error";

interface UpdaterContextValue {
  phase: UpdatePhase;
  pendingUpdate: Update | null;
  errorMsg: string;
  /** 手动触发一次检查；启动 auto-check 也走它 */
  checkNow: () => Promise<void>;
}

const UpdaterContext = createContext<UpdaterContextValue | null>(null);

const intervalToMs: Record<string, number> = {
  daily: 24 * 60 * 60 * 1000,
  weekly: 7 * 24 * 60 * 60 * 1000,
  monthly: 30 * 24 * 60 * 60 * 1000,
};

function shouldAutoCheck(
  enabled: boolean,
  interval: string,
  lastCheckAt: string | null,
): boolean {
  if (!enabled) return false;
  if (interval === "onstartup") return true;
  if (!lastCheckAt) return true;
  const elapsed = Date.now() - new Date(lastCheckAt).getTime();
  return elapsed >= (intervalToMs[interval] ?? intervalToMs.weekly);
}

export function UpdaterProvider({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  const { settings, update: updateSettings } = useSettings();
  const [phase, setPhase] = useState<UpdatePhase>("idle");
  const [pendingUpdate, setPendingUpdate] = useState<Update | null>(null);
  const [errorMsg, setErrorMsg] = useState("");
  // 启动 auto-check 只跑一次，跨 strict-mode 双 mount / settings 多次刷新都不重复
  const startupRanRef = useRef(false);

  const checkNow = useCallback(async () => {
    setPhase("checking");
    setErrorMsg("");
    try {
      const upd = await check();
      // 不管有没有 update，都更新 lastCheck，避免下次启动又立刻查
      updateSettings({ lastUpdateCheckAt: new Date().toISOString() });
      if (upd) {
        setPendingUpdate(upd);
        setPhase("idle");
      } else {
        setPhase("uptodate");
      }
    } catch (e) {
      setPhase("error");
      setErrorMsg(e instanceof Error ? e.message : String(e));
    }
  }, [updateSettings]);

  // 启动时按设置触发一次自动检查
  useEffect(() => {
    if (!settings) return;
    if (startupRanRef.current) return;
    if (
      shouldAutoCheck(
        settings.autoUpdateEnabled,
        settings.autoUpdateInterval,
        settings.lastUpdateCheckAt,
      )
    ) {
      startupRanRef.current = true;
      void checkNow();
    } else {
      // 不需要 auto-check 也算启动判定完成，避免后续 settings 改动再触发
      startupRanRef.current = true;
    }
  }, [settings, checkNow]);

  const confirmInstall = useCallback(async () => {
    const upd = pendingUpdate;
    if (!upd) return;
    setPendingUpdate(null);
    setPhase("installing");
    try {
      await upd.downloadAndInstall();
      await relaunch();
    } catch (e) {
      setPhase("error");
      setErrorMsg(e instanceof Error ? e.message : String(e));
    }
  }, [pendingUpdate]);

  const dismiss = useCallback(() => setPendingUpdate(null), []);

  const value = useMemo<UpdaterContextValue>(
    () => ({ phase, pendingUpdate, errorMsg, checkNow }),
    [phase, pendingUpdate, errorMsg, checkNow],
  );

  return (
    <UpdaterContext.Provider value={value}>
      {children}
      <ConfirmDialog
        open={pendingUpdate !== null}
        title={t("settings.about.update.dialog.title", {
          version: pendingUpdate?.version ?? "",
        })}
        message={t("settings.about.update.dialog.message", {
          body:
            pendingUpdate?.body ||
            t("settings.about.update.dialog.bodyEmpty"),
        })}
        confirmLabel={t("settings.about.update.dialog.confirm")}
        cancelLabel={t("settings.about.update.dialog.cancel")}
        variant="primary"
        onConfirm={confirmInstall}
        onCancel={dismiss}
      />
    </UpdaterContext.Provider>
  );
}

// hook 跟 Provider 同文件是有意为之——消费方一次 import 解决，dev 期 fast refresh
// 在改动 Provider 时退化为整页刷新（state 文件极少改动，影响可接受）。
// eslint-disable-next-line react-refresh/only-export-components
export function useUpdater() {
  const ctx = useContext(UpdaterContext);
  if (!ctx)
    throw new Error("useUpdater must be used within UpdaterProvider");
  return ctx;
}
