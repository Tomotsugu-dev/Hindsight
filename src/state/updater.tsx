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
import { platform } from "@tauri-apps/plugin-os";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ConfirmDialog } from "../components/ConfirmDialog/ConfirmDialog";
import { useSettings } from "./settings";

const RELEASES_URL = "https://github.com/Tomotsugu-dev/Hindsight/releases";

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
  const { settings, update: updateSettings } = useSettings();
  const [phase, setPhase] = useState<UpdatePhase>("idle");
  const [pendingUpdate, setPendingUpdate] = useState<Update | null>(null);
  const [errorMsg, setErrorMsg] = useState("");
  const [isMacOS, setIsMacOS] = useState(false);
  // 启动 auto-check 只跑一次，跨 strict-mode 双 mount / settings 多次刷新都不重复
  const startupRanRef = useRef(false);

  useEffect(() => {
    setIsMacOS(platform() === "macos");
  }, []);

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
    if (isMacOS) {
      // macOS 没付 Apple Developer，应用内静默替换会破 codesign，跳浏览器
      await openUrl(`${RELEASES_URL}/tag/v${upd.version}`);
      return;
    }
    setPhase("installing");
    try {
      await upd.downloadAndInstall();
      await relaunch();
    } catch (e) {
      setPhase("error");
      setErrorMsg(e instanceof Error ? e.message : String(e));
    }
  }, [pendingUpdate, isMacOS]);

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
        title={`发现新版本 v${pendingUpdate?.version ?? ""}`}
        message={
          isMacOS
            ? `点击"前往下载"将打开浏览器到 GitHub Releases 页，请手动下载新版 .dmg 安装。\n\n更新说明：\n${pendingUpdate?.body || "（无）"}`
            : `点击"现在更新"将下载并自动安装新版本，完成后应用会重启。\n\n更新说明：\n${pendingUpdate?.body || "（无）"}`
        }
        confirmLabel={isMacOS ? "前往下载" : "现在更新"}
        cancelLabel="稍后"
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
