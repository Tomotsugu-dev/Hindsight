import { forwardRef, useEffect, useState } from "react";
import {
  MessageSquare,
  RefreshCw,
  Scale,
  Sparkles,
  User,
  type LucideProps,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { platform } from "@tauri-apps/plugin-os";
import { getVersion } from "@tauri-apps/api/app";
import { Section } from "../components/Section";
import { Row } from "../components/Row";
import { ConfirmDialog } from "../../../components/ConfirmDialog/ConfirmDialog";
import styles from "./AboutTab.module.css";

const REPO_URL = "https://github.com/Tomotsugu-dev/Hindsight";
const ISSUES_URL = "https://github.com/Tomotsugu-dev/Hindsight/issues";
const RELEASES_URL = "https://github.com/Tomotsugu-dev/Hindsight/releases";

const openExternal = (e: React.MouseEvent, url: string) => {
  e.preventDefault();
  void openUrl(url).catch(() => {});
};

type Phase = "idle" | "checking" | "uptodate" | "installing" | "error";

/** GitHub Octocat 标记 —— lucide v0.300+ 移除了 brand icon，自己塞一个 */
const GithubMark = forwardRef<SVGSVGElement, LucideProps>(
  ({ size = 16, ...rest }, ref) => (
    <svg
      ref={ref}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden="true"
      {...rest}
    >
      <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" />
    </svg>
  ),
);
GithubMark.displayName = "GithubMark";

export default function AboutTab() {
  const [appVersion, setAppVersion] = useState<string>("");
  const [isMacOS, setIsMacOS] = useState(false);
  const [phase, setPhase] = useState<Phase>("idle");
  const [errorMsg, setErrorMsg] = useState<string>("");
  const [pendingUpdate, setPendingUpdate] = useState<Update | null>(null);

  useEffect(() => {
    void getVersion().then(setAppVersion).catch(() => {});
    setIsMacOS(platform() === "macos");
  }, []);

  const handleCheck = async () => {
    setPhase("checking");
    setErrorMsg("");
    try {
      const update = await check();
      if (update) {
        // 检测到新版本：先把 update 存住，然后弹 ConfirmDialog 让用户决定是否下载安装
        setPendingUpdate(update);
        setPhase("idle");
      } else {
        setPhase("uptodate");
      }
    } catch (e) {
      setPhase("error");
      setErrorMsg(e instanceof Error ? e.message : String(e));
    }
  };

  const handleConfirmUpdate = async () => {
    const update = pendingUpdate;
    if (!update) return;
    setPendingUpdate(null);
    // macOS 占位：当前没用 Apple Developer 签名/公证，应用内静默替换会破坏
    // codesign。直接跳到 Releases 那个 tag 让用户手动下载新 .dmg。
    if (isMacOS) {
      await openUrl(`${RELEASES_URL}/tag/v${update.version}`);
      return;
    }
    setPhase("installing");
    try {
      await update.downloadAndInstall();
      await relaunch();
    } catch (e) {
      setPhase("error");
      setErrorMsg(e instanceof Error ? e.message : String(e));
    }
  };

  const cancelUpdate = () => setPendingUpdate(null);

  const checkBtnDisabled = phase === "checking" || phase === "installing";
  const statusText =
    phase === "checking"
      ? "正在检查…"
      : phase === "uptodate"
        ? "已是最新版本"
        : phase === "installing"
          ? "下载并安装中，完成后将自动重启…"
          : phase === "error"
            ? `检查失败：${errorMsg}`
            : isMacOS
              ? "macOS 暂不支持应用内自动更新，发现新版会跳转到 Releases 页"
              : undefined;

  return (
    <>
      <div className={styles.hero}>
        <div className={styles.logo} aria-hidden />
        <div className={styles.heroText}>
          <div className={styles.appName}>Hindsight</div>
          <div className={styles.version}>
            {appVersion || "0.1.0"} · Tauri 2 + React
          </div>
        </div>
      </div>

      <Section title="应用更新" icon={Sparkles}>
        <Row label="当前版本" description={statusText}>
          <span className={styles.value}>{appVersion || "—"}</span>
          <button
            type="button"
            className={styles.checkBtn}
            onClick={handleCheck}
            disabled={checkBtnDisabled}
          >
            <RefreshCw
              size={13}
              strokeWidth={1.85}
              className={
                phase === "checking" || phase === "installing"
                  ? styles.spinning
                  : ""
              }
            />
            {phase === "checking"
              ? "检查中…"
              : phase === "installing"
                ? "更新中…"
                : "检查更新"}
          </button>
        </Row>
      </Section>

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
        onConfirm={handleConfirmUpdate}
        onCancel={cancelUpdate}
      />

      <Section title="信息">
        <Row label="作者" description="个人项目，非商用" icon={User}>
          <span className={styles.value}>Tomotsugu-dev</span>
        </Row>
        <Row label="许可证" icon={Scale}>
          <span className={styles.value}>MIT</span>
        </Row>
      </Section>

      <Section title="链接">
        <Row label="GitHub 仓库" icon={GithubMark}>
          <a
            href={REPO_URL}
            className={styles.link}
            onClick={(e) => openExternal(e, REPO_URL)}
          >
            查看 →
          </a>
        </Row>
        <Row label="反馈与建议" icon={MessageSquare}>
          <a
            href={ISSUES_URL}
            className={styles.link}
            onClick={(e) => openExternal(e, ISSUES_URL)}
          >
            提交 →
          </a>
        </Row>
      </Section>
    </>
  );
}
