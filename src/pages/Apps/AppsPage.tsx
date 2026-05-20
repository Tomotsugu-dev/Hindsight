import { useTranslation } from "react-i18next";
import { Info } from "lucide-react";
import { PairingSection } from "../Categories/PairingSection";
import styles from "../Categories/Categories.module.css";

/**
 * 应用页：跨设备应用合并 + 分类指派。
 *
 * 同一应用在不同 OS 上进程名不同（macOS 的 "Code" / Windows 的 "Visual Studio
 * Code"）—— 这页把它们拖到同一行就统一名字 / 分类 / 图标 / 时长。
 *
 * 原来是 /categories 下的 pairing tab，现在拆成 sidebar 一级入口「应用」，
 * 跟「分类」(/categories) 平起平坐——分类管"桶"、应用管"放进桶里的东西"。
 *
 * 沿用 Categories 那套 styles + i18n 文案（categories.pairing.*）以减少改动；
 * PairingSection 本体还住在 Categories 目录下，因为跟 parts.tsx 的 AssignDropdown
 * 共享。
 */
export default function AppsPage() {
  const { t } = useTranslation();
  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <div className={styles.headerText}>
          <h1 className={styles.title}>
            {t("apps.title")}
            <button
              type="button"
              className={styles.infoTip}
              aria-label={t("categories.pairing.infoTipAria")}
            >
              <Info size={14} strokeWidth={2.25} />
              <span className={styles.infoTipBody} role="tooltip">
                {t("categories.pairing.infoTipBody")}
              </span>
            </button>
          </h1>
          <p className={styles.meta}>
            {t("categories.pairing.instructionPrefix")}
            <strong className={styles.metaEmph}>
              {t("categories.pairing.instructionEmph")}
            </strong>
            {t("categories.pairing.instructionSuffix")}
            <span className={styles.metaUnassigned}>
              {t("categories.pairing.unassignedHint")}
            </span>
          </p>
        </div>
      </header>

      <section className={styles.card}>
        <PairingSection />
      </section>
    </div>
  );
}
