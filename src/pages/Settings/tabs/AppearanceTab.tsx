import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Palette } from "lucide-react";
import { Section } from "../../../components/FormLayout/Section";
import {
  APP_THEMES,
  getStoredTheme,
  setStoredTheme,
  type AppTheme,
} from "../../../lib/theme";
import styles from "./AppearanceTab.module.css";

export default function AppearanceTab() {
  const { t } = useTranslation();
  // 主题是纯前端偏好（localStorage），不走后端 settings；本地 state 仅用于高亮选中项
  const [theme, setTheme] = useState<AppTheme>(getStoredTheme);

  const choose = (next: AppTheme) => {
    setStoredTheme(next); // 立即写 <html data-theme> + localStorage
    setTheme(next);
  };

  return (
    <Section
      title={t("settings.appearance.sectionTitle")}
      description={t("settings.appearance.description")}
      icon={Palette}
      bare
    >
      <div className={styles.grid}>
        {APP_THEMES.map((key) => {
          const active = key === theme;
          return (
            <button
              key={key}
              type="button"
              className={`${styles.card} ${active ? styles.cardActive : ""}`}
              onClick={() => choose(key)}
              aria-pressed={active}
            >
              <span
                className={`${styles.preview} ${styles[`preview_${key}`]}`}
                aria-hidden
              >
                <span className={styles.previewSidebar} />
                <span className={styles.previewBody}>
                  <span className={styles.previewLine} />
                  <span className={styles.previewLine} />
                  <span className={styles.previewCard} />
                </span>
              </span>
              <span className={styles.meta}>
                <span className={styles.name}>
                  {t(`settings.appearance.themes.${key}.name`)}
                  {active ? (
                    <Check
                      size={14}
                      strokeWidth={2.4}
                      className={styles.check}
                    />
                  ) : null}
                </span>
                <span className={styles.desc}>
                  {t(`settings.appearance.themes.${key}.desc`)}
                </span>
              </span>
            </button>
          );
        })}
      </div>
    </Section>
  );
}
