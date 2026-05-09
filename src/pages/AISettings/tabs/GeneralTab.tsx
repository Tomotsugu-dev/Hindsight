import { useTranslation } from "react-i18next";
import { Clock, Filter, Image as ImageIcon } from "lucide-react";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { Slider } from "../../../components/FormControls/Slider";
import { SegmentList } from "../../../components/FormControls/SegmentList";
import { CategoryChipMultiSelect } from "../../../components/FormControls/CategoryChipMultiSelect";
import { type AiSegment } from "../../../api/hindsight";
import { useAiSettings } from "../shared/useAiSettings";
import styles from "../AISettings.module.css";

/**
 * AI 设置 → 常规 tab：决定"喂给 LLM 的数据范围"——时段切分 + 分类过滤 + 抽帧参数。
 * 这三件事都是 prompt 之前的数据加工，跟"怎么写总结"（PromptTab）解耦。
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

      <Section
        title={t("aiSettings.frame.sectionTitle")}
        icon={ImageIcon}
        description={t("aiSettings.frame.sectionDesc")}
      >
        <Row
          label={t("aiSettings.frame.dedupThresholdLabel")}
          labelHint={t("aiSettings.frame.dedupThresholdHint")}
        >
          {/* 后端存 0.70..=0.99，UI 用 70..=99 整数百分比，存取时 × 100 / ÷ 100 */}
          <Slider
            value={Math.round(ai.dedupThreshold * 100)}
            onChange={(v) => updateAi({ dedupThreshold: v / 100 })}
            min={70}
            max={99}
            step={1}
            suffix="%"
          />
        </Row>
        <Row
          label={t("aiSettings.frame.maxImagesLabel")}
          labelHint={t("aiSettings.frame.maxImagesHint")}
        >
          <Slider
            value={ai.maxImagesPerSegment}
            onChange={(v) => updateAi({ maxImagesPerSegment: v })}
            min={1}
            max={5000}
            step={10}
          />
        </Row>
      </Section>
    </div>
  );
}
