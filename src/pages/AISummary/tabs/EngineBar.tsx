import { useTranslation } from "react-i18next";
import type { EngineStatus } from "../../../api/hindsight";
import { humanAccelLabel } from "./debugTabHelpers";
import styles from "./DebugTab.module.css";

/** 引擎状态条：端口 / 模型 / ctx / 状态指示 dot。 */
export function EngineBar({ engine }: { engine: EngineStatus | null }) {
  const { t } = useTranslation();
  if (!engine) {
    return (
      <div className={styles.engineBar}>
        <span className={styles.engineDot} />
        <span>{t("aiSummary.debug.engineBar.loading")}</span>
      </div>
    );
  }
  const rt = engine.runtime;
  const dotClass =
    rt.state === "running"
      ? styles.engineDotRunning
      : rt.state === "starting"
        ? styles.engineDotStarting
        : rt.state === "error"
          ? styles.engineDotError
          : "";
  const versionStr = engine.installed
    ? engine.installedVersion ?? engine.currentPin
    : t("aiSummary.debug.engineBar.notInstalled");
  return (
    <div className={styles.engineBar}>
      <span className={`${styles.engineDot} ${dotClass}`} />
      <span>
        {t("aiSummary.debug.engineBar.port")}
        <span className={styles.engineMetaStrong}>
          {rt.state === "running" && rt.port != null ? `:${rt.port}` : "—"}
        </span>
      </span>
      <span className={styles.engineSep}>·</span>
      <span>
        {t("aiSummary.debug.engineBar.version")}
        <span className={styles.engineMetaStrong}>{versionStr}</span>
      </span>
      <span className={styles.engineSep}>·</span>
      <span>
        {t("aiSummary.debug.engineBar.accel")}
        <span
          className={styles.engineMetaStrong}
          title={t("aiSummary.debug.engineBar.accelTitle", { id: engine.platformId })}
        >
          {humanAccelLabel(engine.platformId)}
        </span>
      </span>
      <span className={styles.engineSep}>·</span>
      <span>
        {t("aiSummary.debug.engineBar.state")}
        <span className={styles.engineMetaStrong}>{rt.state}</span>
      </span>
      {rt.error ? (
        <>
          <span className={styles.engineSep}>·</span>
          <span style={{ color: "#dc2626" }}>
            {t("aiSummary.debug.engineBar.error")}
            {rt.error}
          </span>
        </>
      ) : null}
    </div>
  );
}
