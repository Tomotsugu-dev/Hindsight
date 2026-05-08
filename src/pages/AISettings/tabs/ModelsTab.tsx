import { useTranslation } from "react-i18next";
import { Bot } from "lucide-react";
import { Section } from "../../../components/FormLayout/Section";
import { ModelsSection } from "../shared/ModelsSection";
import styles from "../AISettings.module.css";

/**
 * AI 设置 → 模型 tab：本地 GGUF 模型管理 + step 分配。
 *
 * step 1 / step 2 的分配在每张已下载卡片上的 toggle 完成（vision 字段决定 step 1 是否可点）。
 */
export default function ModelsTab() {
  const { t } = useTranslation();
  return (
    <div className={styles.content}>
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
