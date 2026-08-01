import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { CalendarClock, Clock, Filter, Minus, Plus, ScanText, Timer } from "lucide-react";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { Toggle } from "../../../components/FormControls/Toggle";
import { TimeOfDayPicker } from "../../../components/FormControls/TimeOfDayPicker";
import { SegmentList } from "../../../components/FormControls/SegmentList";
import { CategoryChipMultiSelect } from "../../../components/FormControls/CategoryChipMultiSelect";
import { type AiSegment } from "../../../api/hindsight";
import { useAiSettings } from "../shared/useAiSettings";
import { useSettings } from "../../../state/settings";
import {
  formatHour,
  nextFreeHour,
  parseHour,
} from "../../../components/FormControls/timeOfDayMath";
import styles from "../AISettings.module.css";

/**
 * AI 设置 → 常规 tab：决定"喂给 LLM 的数据范围"——时段切分 + 分类过滤。
 * 这两件事都是 prompt 之前的数据加工，跟"怎么写总结"（PromptTab）解耦。
 */
export default function GeneralTab() {
  const { t } = useTranslation();
  const { ai, updateAi } = useAiSettings();
  // 定时补识别是顶层 settings 字段(屏幕记忆域),不走 ai 子配置
  const { settings, update } = useSettings();
  // legacy 自愈:此前"自动总结开 + 未指定时刻 = 尽快"的老配置,
  // 首次进入本页补写默认 23:00,让滑条显示与实际行为一致
  useEffect(() => {
    if (
      ai?.autoSummary &&
      ai.autoSummaryTimes.length === 0 &&
      ai.autoSummaryAt == null
    ) {
      updateAi({ autoSummaryTimes: ["23:00"] });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ai?.autoSummary, ai?.autoSummaryAt, ai?.autoSummaryTimes]);
  if (!ai || !settings) return null;
  // 与后端 effective_auto_summary_times 同口径:新列表优先,空回落旧单字段
  const summaryTimes =
    ai.autoSummaryTimes.length > 0
      ? ai.autoSummaryTimes
      : ai.autoSummaryAt != null
        ? [ai.autoSummaryAt]
        : [];
  // 与后端 effective_ocr_daily_times 同口径:新列表优先,空则回落旧单时刻字段
  const ocrTimes =
    settings.memoryOcrDailyTimes.length > 0
      ? settings.memoryOcrDailyTimes
      : settings.memoryOcrDailyAt != null
        ? [settings.memoryOcrDailyAt]
        : [];

  return (
    <div className={styles.content}>
      <Section
        title={t("aiSettings.autoSummary.sectionTitle")}
        icon={Timer}
        description={t("aiSettings.autoSummary.sectionDesc")}
      >
        <Row
          icon={CalendarClock}
          label={t("aiSettings.autoSummary.enableLabel")}
          description={t("aiSettings.autoSummary.enableHint")}
        >
          <Toggle
            checked={ai.autoSummary}
            onChange={(next) =>
              updateAi({
                autoSummary: next,
                // 开启即定下默认时刻(23:00):行为与时间轴显示保持一致
                ...(next && summaryTimes.length === 0
                  ? { autoSummaryTimes: ["23:00"], autoSummaryAt: null }
                  : {}),
              })
            }
            ariaLabel={t("aiSettings.autoSummary.enableLabel")}
          />
        </Row>
        {ai.autoSummary && (
          <Row
            label={t("aiSettings.autoSummary.timeLabel")}
            block
            actions={
              <>
                <button
                  type="button"
                  className={styles.timeDotBtn}
                  onClick={() =>
                    updateAi({
                      autoSummaryTimes: [
                        ...summaryTimes,
                        formatHour(nextFreeHour(summaryTimes.map(parseHour))),
                      ],
                      autoSummaryAt: null,
                    })
                  }
                  disabled={summaryTimes.length >= 6}
                  aria-label={t("aiSettings.autoSummary.addTimeAria")}
                >
                  <Plus size={12} strokeWidth={2.25} />
                </button>
                <button
                  type="button"
                  className={styles.timeDotBtn}
                  onClick={() =>
                    updateAi({
                      autoSummaryTimes: summaryTimes.slice(0, -1),
                      autoSummaryAt: null,
                    })
                  }
                  disabled={summaryTimes.length <= 1}
                  aria-label={t("aiSettings.autoSummary.removeTimeAria")}
                >
                  <Minus size={12} strokeWidth={2.25} />
                </button>
              </>
            }
          >
            <TimeOfDayPicker
              values={summaryTimes.length > 0 ? summaryTimes : ["23:00"]}
              onChange={(next) =>
                updateAi({ autoSummaryTimes: next, autoSummaryAt: null })
              }
              ariaLabel={t("aiSettings.autoSummary.timeLabel")}
              bands={ai.segments}
            />
          </Row>
        )}
        <Row
          icon={ScanText}
          label={t("aiSettings.autoSummary.ocrDailyLabel")}
          description={t("aiSettings.autoSummary.ocrDailyDescription")}
          disabled={!settings.captureEnabled || !settings.screenshotEnabled}
        >
          <Toggle
            checked={ocrTimes.length > 0}
            onChange={(next) =>
              // 默认 22:00:先于自动总结的默认 23:00——先清积压,总结用上最新索引
              update({ memoryOcrDailyTimes: next ? ["22:00"] : [] })
            }
            ariaLabel={t("aiSettings.autoSummary.ocrDailyLabel")}
          />
        </Row>
        {ocrTimes.length > 0 && (
          <Row
            label={t("aiSettings.autoSummary.timeLabel")}
            block
            actions={
              <>
                <button
                  type="button"
                  className={styles.timeDotBtn}
                  onClick={() =>
                    update({
                      memoryOcrDailyTimes: [
                        ...ocrTimes,
                        formatHour(nextFreeHour(ocrTimes.map(parseHour))),
                      ],
                    })
                  }
                  disabled={ocrTimes.length >= 6}
                  aria-label={t("aiSettings.autoSummary.addTimeAria")}
                >
                  <Plus size={12} strokeWidth={2.25} />
                </button>
                <button
                  type="button"
                  className={styles.timeDotBtn}
                  onClick={() =>
                    update({ memoryOcrDailyTimes: ocrTimes.slice(0, -1) })
                  }
                  disabled={ocrTimes.length <= 1}
                  aria-label={t("aiSettings.autoSummary.removeTimeAria")}
                >
                  <Minus size={12} strokeWidth={2.25} />
                </button>
              </>
            }
          >
            <TimeOfDayPicker
              values={ocrTimes}
              onChange={(next) => update({ memoryOcrDailyTimes: next })}
              ariaLabel={t("aiSettings.autoSummary.ocrDailyLabel")}
              bands={ai.segments}
            />
          </Row>
        )}
      </Section>

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
