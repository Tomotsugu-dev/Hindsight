import { forwardRef, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Info,
  Link2,
  MessageSquare,
  RefreshCw,
  Scale,
  User,
  type LucideProps,
} from "lucide-react";
import { UpdateIcon } from "../../../components/icons/UpdateIcon";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getVersion } from "@tauri-apps/api/app";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { Toggle } from "../../../components/FormControls/Toggle";
import { SimplePicker } from "../../../components/SimplePicker/SimplePicker";
import { useSettings } from "../../../state/settings";
import { useUpdater } from "../../../state/updater";
import logoUrl from "../../../assets/logo.png";
import styles from "./AboutTab.module.css";

const REPO_URL = "https://github.com/Tomotsugu-dev/Hindsight";
const ISSUES_URL = "https://github.com/Tomotsugu-dev/Hindsight/issues";

const openExternal = (e: React.MouseEvent, url: string) => {
  e.preventDefault();
  void openUrl(url).catch(() => {});
};

type UpdateInterval = "daily" | "weekly" | "monthly" | "onstartup";

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
  const { t } = useTranslation();
  const [appVersion, setAppVersion] = useState<string>("");
  const { settings, update: updateSettings } = useSettings();
  const { phase, errorMsg, checkNow } = useUpdater();

  // 频率下拉选项；label 跟随当前 locale
  const intervalOptions = useMemo<{ value: UpdateInterval; label: string }[]>(
    () => [
      { value: "daily", label: t("settings.about.update.intervals.daily") },
      { value: "weekly", label: t("settings.about.update.intervals.weekly") },
      { value: "monthly", label: t("settings.about.update.intervals.monthly") },
      {
        value: "onstartup",
        label: t("settings.about.update.intervals.onstartup"),
      },
    ],
    [t],
  );

  useEffect(() => {
    void getVersion().then(setAppVersion).catch(() => {});
  }, []);

  if (!settings) return null;

  const checkBtnDisabled = phase === "checking" || phase === "installing";
  const statusText =
    phase === "checking"
      ? t("settings.about.update.status.checking")
      : phase === "uptodate"
        ? t("settings.about.update.status.uptodate")
        : phase === "installing"
          ? t("settings.about.update.status.installing")
          : phase === "error"
            ? t("settings.about.update.status.error", { message: errorMsg })
            : undefined;

  return (
    <>
      <div className={styles.hero}>
        <img
          className={styles.logo}
          src={logoUrl}
          alt=""
          aria-hidden
          draggable={false}
        />
        <div className={styles.heroText}>
          <div className={styles.appName}>Hindsight</div>
          <div className={styles.version}>
            {t("settings.about.subtitle", {
              version: appVersion || "0.1.0",
            })}
          </div>
        </div>
      </div>

      <Section title={t("settings.about.update.title")} icon={UpdateIcon}>
        <Row
          label={t("settings.about.update.currentVersionLabel")}
          description={statusText}
        >
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
              ? t("settings.about.update.checkingBtn")
              : phase === "installing"
                ? t("settings.about.update.installingBtn")
                : t("settings.about.update.checkBtn")}
          </button>
        </Row>

        <Row
          label={t("settings.about.update.autoLabel")}
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
            <Row label={t("settings.about.update.intervalLabel")}>
              <SimplePicker
                value={settings.autoUpdateInterval}
                options={intervalOptions}
                onChange={(v) =>
                  updateSettings({
                    autoUpdateInterval: v,
                  })
                }
              />
            </Row>
          </div>
        </div>
      </Section>

      <Section title={t("settings.about.info.title")} icon={Info}>
        <Row
          label={t("settings.about.info.authorLabel")}
          icon={User}
        >
          <span className={styles.value}>Tomotsugu-dev</span>
        </Row>
        <Row label={t("settings.about.info.licenseLabel")} icon={Scale}>
          <span className={styles.value}>MIT</span>
        </Row>
      </Section>

      <Section title={t("settings.about.links.title")} icon={Link2}>
        <Row label={t("settings.about.links.repoLabel")} icon={GithubMark}>
          <a
            href={REPO_URL}
            className={styles.link}
            onClick={(e) => openExternal(e, REPO_URL)}
          >
            {t("settings.about.links.repoLink")}
          </a>
        </Row>
        <Row label={t("settings.about.links.feedbackLabel")} icon={MessageSquare}>
          <a
            href={ISSUES_URL}
            className={styles.link}
            onClick={(e) => openExternal(e, ISSUES_URL)}
          >
            {t("settings.about.links.feedbackLink")}
          </a>
        </Row>
      </Section>
    </>
  );
}
