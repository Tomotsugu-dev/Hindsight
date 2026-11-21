import { useTranslation } from "react-i18next";
import { DownloadSourcesLink } from "../shared/DownloadSourcesSection";
import { Bot } from "lucide-react";
import { Section } from "../../../components/FormLayout/Section";
import { ReportModelSection } from "../../../components/ReportModel/ReportModelSection";
import { ModelsSection } from "../shared/ModelsSection";
import styles from "../AISettings.module.css";

/**
 * AI 设置 → 模型 tab：最上面选日报、周报用哪个模型和思考模式，下面管理本地 GGUF 模型。
 */
export default function ModelsTab() {
  const { t } = useTranslation();
  return (
    <div className={styles.content}>
      <ReportModelSection />
      <DownloadSourcesLink />
      <Section
        title={t("aiSettings.models.sectionTitle")}
        description={t("aiSettings.models.sectionDesc")}
        icon={Bot}
      >
        <ModelsSection />
      </Section>
    </div>
  );
}
