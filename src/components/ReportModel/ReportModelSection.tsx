import { useTranslation } from "react-i18next";
import { Brain, Cloud, HardDrive, Link2, Sparkles } from "lucide-react";
import { thinkingLabelKey } from "../../lib/thinkingModes";
import { FOLLOW_DAILY, useReportModel, type ReportModule } from "../../state/reportModel";
import { CalendarTodayIcon } from "../icons/CalendarTodayIcon";
import { CalendarWeekIcon } from "../icons/CalendarWeekIcon";
import { Row } from "../FormLayout/Row";
import { Section } from "../FormLayout/Section";
import { SimplePicker } from "../SimplePicker/SimplePicker";
import { ReportCloudConfirm } from "./ReportCloudConfirm";
import { checkedModel, modelOptions, routeLabel } from "./reportModelOptions";
import styles from "./ReportModelSection.module.css";

/** 下拉框里「还没选」的值，只在一项都对不上时出现 */
const NONE = "\u0000none";

/** AI 设置 → 模型 顶部：日报、周报各用哪个模型、什么思考模式。和报告页工具栏上的是同一个设置。 */
export function ReportModelSection() {
  const { t } = useTranslation();
  return (
    <Section
      title={t("aiSettings.models.reports.title")}
      description={t("aiSettings.models.reports.description")}
      icon={Sparkles}
    >
      <ReportModelRow module="daily" />
      <ReportModelRow module="weekly" />
    </Section>
  );
}

function ReportModelRow({ module }: { module: ReportModule }) {
  const { t } = useTranslation();
  const state = useReportModel(module);
  if (!state) return null;

  const label = t(module === "daily" ? "aiSummary.tabs.daily" : "aiSummary.tabs.week");
  const options = modelOptions(state, module, t);
  const checked = checkedModel(state, module);
  const following = module === "weekly" && state.value === FOLLOW_DAILY;
  const modelIcon = following ? Link2 : state.route.cloud ? Cloud : HardDrive;

  return (
    <Row
      label={label}
      icon={module === "daily" ? CalendarTodayIcon : CalendarWeekIcon}
      // 跟日报时下拉框上只写「跟日报一样」，这里写出实际用的模型
      description={
        following
          ? t("aiSettings.models.reports.inUse", { model: routeLabel(state, t) })
          : undefined
      }
    >
      <div className={styles.controls}>
        {options.length === 0 ? (
          <span className={styles.empty}>{t("chat.model.empty")}</span>
        ) : (
          <SimplePicker
            value={checked ?? NONE}
            options={
              checked == null ? [{ value: NONE, label: t("chat.badge.none") }, ...options] : options
            }
            onChange={(v) => {
              if (v !== NONE) state.chooseModel(v);
            }}
            icon={modelIcon}
            size="md"
            title={`${label} · ${t("aiSummary.model.modelGroup")}`}
          />
        )}
        <SimplePicker
          value={state.thinking}
          options={state.modes.map((m) => ({
            value: m,
            label: t(thinkingLabelKey(m, state.modes)),
          }))}
          onChange={state.chooseThinking}
          icon={Brain}
          size="md"
          title={`${label} · ${t("chat.thinkingMode.label")}`}
        />
      </div>
      <ReportCloudConfirm state={state} />
    </Row>
  );
}
