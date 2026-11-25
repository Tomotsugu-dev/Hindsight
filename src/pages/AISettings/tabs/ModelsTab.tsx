import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";
import { ReportModelSection } from "../../../components/ReportModel/ReportModelSection";
import { ROUTES } from "../../../config/nav";
import { EngineParamsSection } from "../shared/EngineParamsSection";
import styles from "../AISettings.module.css";

/**
 * AI settings → Models: which model and thinking mode the daily and weekly reports use.
 *
 * When a report uses a local model, its engine parameters follow. Downloading local models and
 * adding cloud APIs are in Settings → AI models. The line at the bottom links there.
 */
export default function ModelsTab() {
  const { t } = useTranslation();
  return (
    <div className={styles.content}>
      <ReportModelSection />
      <EngineParamsSection />
      <p className={styles.manageModelsLink}>
        <span>{t("aiSettings.models.manageHint")}</span>
        <Link to={ROUTES.settingsAiModels}>{t("aiSettings.models.manageLink")}</Link>
      </p>
    </div>
  );
}
