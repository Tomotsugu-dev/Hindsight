import { useTranslation } from "react-i18next";
import { Info } from "lucide-react";
import { PairingSection } from "../PairingSection";
import styles from "../Categories.module.css";

/** 多设备合并 tab：把不同设备上同一个 app（process 名不同）合并到一个 app group。
 *  原 CategoriesPage 的 Section 2 提取过来，header 描述文案沿用 categories.pairing.* */
export default function PairingTab() {
  const { t } = useTranslation();
  return (
    <>
      <header className={styles.header}>
        <div className={styles.headerText}>
          <h2 className={styles.title} style={{ fontSize: 18 }}>
            {t("categories.pairing.sectionTitle")}
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
          </h2>
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
    </>
  );
}
