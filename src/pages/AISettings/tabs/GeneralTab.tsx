import { useTranslation } from "react-i18next";
import { Clock, Filter } from "lucide-react";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { SegmentList } from "../../../components/FormControls/SegmentList";
import { CategoryChipMultiSelect } from "../../../components/FormControls/CategoryChipMultiSelect";
import { type AiSegment } from "../../../api/hindsight";
import { useAiSettings } from "../shared/useAiSettings";
import styles from "../AISettings.module.css";

/**
 * AI 设置 → 常规 tab：决定"喂给 LLM 的数据范围"——时段切分 + 分类过滤。
 * 这两件事都是 prompt 之前的数据加工，跟"怎么写总结"（PromptTab）解耦。
 */
export default function GeneralTab() {
  const { t } = useTranslation();
  const { ai, updateAi } = useAiSettings();
  if (!ai) return null;

  return (
    <div className={styles.content}>
      <Section
        title={t("aiSettings.segments.sectionTitle")}
        icon={Clock}
        info={t("aiSettings.segments.sectionInfo")}
      >
        <Row label={t("aiSettings.segments.rowLabel")} block>
          <SegmentList
            segments={ai.segments}
            onChange={(next: AiSegment[]) => updateAi({ segments: next })}
          />
        </Row>
      </Section>

      <Section title={t("aiSettings.filter.sectionTitle")} icon={Filter}>
        <Row
          label={t("aiSettings.filter.rowLabel")}
          labelHint={t("aiSettings.filter.rowHint")}
          block
        >
          <CategoryChipMultiSelect
            selectedIds={ai.excludedCategories}
            onChange={(next) => updateAi({ excludedCategories: next })}
          />
        </Row>
      </Section>
    </div>
  );
}
