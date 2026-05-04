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
import { getVersion } from "@tauri-apps/api/app";
import { Section } from "../components/Section";
import { Row } from "../components/Row";
import { Toggle } from "../components/Toggle";
import { SimplePicker } from "../../../components/SimplePicker/SimplePicker";
import { useSettings } from "../../../state/settings";
import { useUpdater } from "../../../state/updater";
import type { Settings } from "../../../api/hindsight";
import styles from "./AboutTab.module.css";

const REPO_URL = "https://github.com/Tomotsugu-dev/Hindsight";
const ISSUES_URL = "https://github.com/Tomotsugu-dev/Hindsight/issues";

const openExternal = (e: React.MouseEvent, url: string) => {
  e.preventDefault();
  void openUrl(url).catch(() => {});
};

const INTERVAL_OPTIONS: { value: "daily" | "weekly" | "monthly" | "onstartup"; label: string }[] = [
  { value: "daily", label: "每天" },
  { value: "weekly", label: "每周" },
  { value: "monthly", label: "每月" },
  { value: "onstartup", label: "每次打开应用时" },
];

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
  const { settings, update: updateSettings } = useSettings();
  const { phase, errorMsg, checkNow } = useUpdater();

  useEffect(() => {
    void getVersion().then(setAppVersion).catch(() => {});
  }, []);

  if (!settings) return null;

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
            onClick={() => void checkNow()}
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

        <Row
          label="自动检查更新"
          description="开启后，应用会按下面设定的频率在启动时静默检查一次，发现新版本会弹出提示。"
        >
          <Toggle
            checked={settings.autoUpdateEnabled}
            onChange={(v) => updateSettings({ autoUpdateEnabled: v })}
          />
        </Row>

        <div
          className={`${styles.collapsible} ${
            settings.autoUpdateEnabled ? styles.collapsibleOpen : ""
          }`}
        >
          <div className={styles.collapsibleInner}>
            <Row label="检查频率">
              <SimplePicker
                value={settings.autoUpdateInterval}
                options={INTERVAL_OPTIONS}
                onChange={(v) =>
                  updateSettings({
                    autoUpdateInterval: v as Settings["autoUpdateInterval"],
                  })
                }
              />
            </Row>
          </div>
        </div>
      </Section>

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
