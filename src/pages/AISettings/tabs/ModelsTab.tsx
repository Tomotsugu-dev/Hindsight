import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Bot, ListTree } from "lucide-react";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { SimplePicker } from "../../../components/SimplePicker/SimplePicker";
import { ModelsSection } from "../shared/ModelsSection";
import { useAiSettings } from "../shared/useAiSettings";
import {
  api,
  SUMMARY_CLOUD_SENTINEL,
  type ModelEntry,
} from "../../../api/hindsight";
import styles from "../AISettings.module.css";

/** 洞察任务分配的 sentinel(前端展示用;洞察目前只有云端·视觉一个来源)。 */
const INSIGHT_CLOUD_VISION = "__cloud_vision__";

/**
 * AI 设置 → 模型 tab:「任务与模型」总览(分配层) + 本地 GGUF 模型管理。
 *
 * 任务分配语义:三行统一 picker——总结/对话在本地模型与云端·文本之间切;
 * 截图洞察目前仅云端·视觉一个来源(本地 VLM 已从架构移除,将来有了选项自然
 * 扩展),其连接配置在「云端 API → 视觉模型」。
 */
export default function ModelsTab() {
  const { t } = useTranslation();
  const { ai, updateAi } = useAiSettings();
  const [locals, setLocals] = useState<ModelEntry[]>([]);

  useEffect(() => {
    api
      .listLocalModels()
      .then((list) => setLocals(list.filter((m) => !m.isMmproj)))
      .catch(() => setLocals([]));
  }, []);

  if (!ai) return null;

  const localOptions = locals.map((m) => ({
    value: m.filename,
    label: m.filename,
  }));
  const cloudTextOption = {
    value: SUMMARY_CLOUD_SENTINEL,
    label: t("aiSettings.models.assign.cloudText"),
  };

  // 空值语义:总结空 = fallback 到 activeMain;对话空 = 自动(云端配好走云端)。
  // picker 显示 effective 值,写回落显式选择——与模型卡上的分配 toggle 同真相源。
  const summaryValue =
    ai.summaryMain.trim() || ai.activeMain.trim() || SUMMARY_CLOUD_SENTINEL;
  const chatValue = ai.chatMain.trim() || SUMMARY_CLOUD_SENTINEL;

  return (
    <div className={styles.content}>
      <Section
        title={t("aiSettings.models.assign.title")}
        description={t("aiSettings.models.assign.desc")}
        icon={ListTree}
      >
        <Row label={t("aiSettings.models.assign.summary")}>
          <SimplePicker<string>
            value={summaryValue}
            options={[cloudTextOption, ...localOptions]}
            onChange={(v) => updateAi({ summaryMain: v })}
          />
        </Row>
        <Row label={t("aiSettings.models.assign.chat")}>
          <SimplePicker<string>
            value={chatValue}
            options={[cloudTextOption, ...localOptions]}
            onChange={(v) => updateAi({ chatMain: v })}
          />
        </Row>
        <Row
          label={t("aiSettings.models.assign.insight")}
          description={
            ai.visionModel.trim()
              ? t("aiSettings.models.assign.insightConfigured", {
                  model: ai.visionModel.trim(),
                })
              : t("aiSettings.models.assign.insightMissing")
          }
        >
          <SimplePicker<string>
            value={INSIGHT_CLOUD_VISION}
            options={[
              {
                value: INSIGHT_CLOUD_VISION,
                label: t("aiSettings.models.assign.cloudVision"),
              },
            ]}
            onChange={() => {}}
          />
        </Row>
      </Section>

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
