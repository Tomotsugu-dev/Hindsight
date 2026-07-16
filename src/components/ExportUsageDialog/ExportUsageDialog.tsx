import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { CheckCircle2, Loader2 } from "lucide-react";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import { useDurationFormatter } from "../../utils/duration";
import { SimplePicker, type SimplePickerOption } from "../SimplePicker/SimplePicker";
import { api } from "../../api/hindsight";
import {
  collectUsageData,
  fmtLocalDate,
  MARKDOWN_TOP_APPS,
  renderUsageExport,
  usageExportFilename,
  type UsageExportFormat,
} from "../../lib/usageExport";
import { logError } from "../../lib/logger";
import styles from "./ExportUsageDialog.module.css";

interface Props {
  open: boolean;
  onClose: () => void;
}

type RangePreset = "last7" | "last30" | "last90" | "custom";

/** 各格式的 save dialog 文件过滤器。 */
const SAVE_FILTERS: Record<UsageExportFormat, { name: string; extensions: string[] }> = {
  csv: { name: "CSV", extensions: ["csv"] },
  json: { name: "JSON", extensions: ["json"] },
  markdown: { name: "Markdown", extensions: ["md"] },
};

/** Markdown 排最前并作为默认：多数用户要的是"能直接看的报告"；CSV / JSON 留给
 *  想自己做分析 / 二次开发的人。 */
const FORMAT_ORDER: UsageExportFormat[] = ["markdown", "csv", "json"];

/** 预设 → 范围（含今天往前 N 天）。 */
function presetRange(preset: Exclude<RangePreset, "custom">): {
  start: string;
  end: string;
} {
  const today = new Date();
  const days = preset === "last7" ? 6 : preset === "last30" ? 29 : 89;
  const start = new Date(today);
  start.setDate(start.getDate() - days);
  return { start: fmtLocalDate(start), end: fmtLocalDate(today) };
}

/**
 * 「导出使用数据」配置弹窗（设置 → 数据 → 存储）。
 * 日期范围（预设 / 自定义）+ 统计粒度（日 / 周 / 月）+ 文件格式（CSV / JSON / Markdown），
 * 确认后走 save dialog 选路径 → 拉统计 → 写盘。只导统计数据，不含原始活动记录
 * （后端也没有暴露原始记录查询）。
 */
export function ExportUsageDialog({ open, onClose }: Props) {
  const { t, i18n } = useTranslation();
  const fmtDuration = useDurationFormatter();

  const [preset, setPreset] = useState<RangePreset>("last30");
  const [customStart, setCustomStart] = useState("");
  const [customEnd, setCustomEnd] = useState("");
  const [daily, setDaily] = useState(true);
  const [weekly, setWeekly] = useState(true);
  const [monthly, setMonthly] = useState(true);
  const [format, setFormat] = useState<UsageExportFormat>("markdown");
  const [busy, setBusy] = useState(false);
  const [donePath, setDonePath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const todayStr = fmtLocalDate(new Date());

  // 每次打开重置：范围回默认，自定义日期预填最近 30 天，清掉上次的结果 / 错误
  useEffect(() => {
    if (open) {
      const def = presetRange("last30");
      setPreset("last30");
      setCustomStart(def.start);
      setCustomEnd(def.end);
      setDaily(true);
      setWeekly(true);
      setMonthly(true);
      setFormat("markdown");
      setBusy(false);
      setDonePath(null);
      setError(null);
    }
  }, [open]);

  const range = preset === "custom" ? { start: customStart, end: customEnd } : presetRange(preset);
  const rangeValid =
    preset !== "custom" ||
    (customStart.length > 0 && customEnd.length > 0 && customStart <= customEnd);
  const anyGranularity = daily || weekly || monthly;
  const canExport = !busy && rangeValid && anyGranularity;

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      // busy 中不许关：writeTextFile 在途，关掉弹窗会丢掉成功 / 失败反馈
      if (e.key === "Escape" && !busy) onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, busy, onClose]);

  const dialogRef = useRef<HTMLDivElement>(null);
  useFocusTrap(open, dialogRef);

  if (!open) return null;

  const runExport = async () => {
    setError(null);
    setDonePath(null);

    // 先选保存位置再拉数据：用户取消 save dialog 时不做无谓查询
    let chosenPath: string | null = null;
    try {
      chosenPath = await save({
        title: t("settings.data.export.dialogTitle"),
        defaultPath: usageExportFilename({ rangeStart: range.start, rangeEnd: range.end }, format),
        filters: [SAVE_FILTERS[format]],
      });
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
      return;
    }
    if (!chosenPath) return; // 用户取消

    setBusy(true);
    try {
      const labels = { t, locale: i18n.language, fmtDuration };
      const data = await collectUsageData(
        { start: range.start, end: range.end, daily, weekly, monthly },
        labels,
      );
      await api.writeTextFile(chosenPath, renderUsageExport(data, format, labels));
      setDonePath(chosenPath);
    } catch (e) {
      logError("data.exportUsage", e);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const revealDone = async () => {
    if (!donePath) return;
    try {
      await revealItemInDir(donePath);
    } catch (e) {
      logError("data.exportUsage", e);
    }
  };

  const presetOptions: SimplePickerOption<RangePreset>[] = [
    { value: "last7", label: t("settings.data.export.presets.last7") },
    { value: "last30", label: t("settings.data.export.presets.last30") },
    { value: "last90", label: t("settings.data.export.presets.last90") },
    { value: "custom", label: t("settings.data.export.presets.custom") },
  ];

  return createPortal(
    <div className={styles.backdrop} onMouseDown={busy ? undefined : onClose} role="presentation">
      {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions */}
      <div
        ref={dialogRef}
        className={styles.dialog}
        role="dialog"
        aria-modal="true"
        aria-labelledby="export-usage-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <h2 id="export-usage-title" className={styles.title}>
          {t("settings.data.export.dialogTitle")}
        </h2>

        {/* ── 日期范围 ── */}
        <div className={styles.field}>
          <span className={styles.fieldLabel}>{t("settings.data.export.rangeLabel")}</span>
          <div className={styles.rangeRow}>
            <SimplePicker
              value={preset}
              options={presetOptions}
              onChange={setPreset}
              disabled={busy}
            />
            {preset === "custom" && (
              <>
                <input
                  type="date"
                  className={styles.dateInput}
                  value={customStart}
                  max={customEnd || todayStr}
                  onChange={(e) => setCustomStart(e.target.value)}
                  disabled={busy}
                  aria-label={t("settings.data.export.customStartAria")}
                />
                <span className={styles.rangeSep}>{t("settings.data.export.customTo")}</span>
                <input
                  type="date"
                  className={styles.dateInput}
                  value={customEnd}
                  min={customStart || undefined}
                  max={todayStr}
                  onChange={(e) => setCustomEnd(e.target.value)}
                  disabled={busy}
                  aria-label={t("settings.data.export.customEndAria")}
                />
              </>
            )}
          </div>
          {!rangeValid && (
            <p className={styles.fieldError}>{t("settings.data.export.invalidRange")}</p>
          )}
        </div>

        {/* ── 导出内容（统计粒度） ── */}
        <div className={styles.field}>
          <span className={styles.fieldLabel}>{t("settings.data.export.contentLabel")}</span>
          <div className={styles.granRow}>
            {(
              [
                ["daily", daily, setDaily],
                ["weekly", weekly, setWeekly],
                ["monthly", monthly, setMonthly],
              ] as const
            ).map(([key, checked, set]) => (
              <label key={key} className={styles.check}>
                <input
                  type="checkbox"
                  checked={checked}
                  onChange={(e) => set(e.target.checked)}
                  disabled={busy}
                />
                {t(`settings.data.export.granularity.${key}`)}
              </label>
            ))}
          </div>
          <p className={styles.fieldHint}>{t("settings.data.export.contentHint")}</p>
          {!anyGranularity && (
            <p className={styles.fieldError}>{t("settings.data.export.needOneGranularity")}</p>
          )}
        </div>

        {/* ── 文件格式（radio 卡片 + 小字区别说明） ── */}
        <div className={styles.field}>
          <span className={styles.fieldLabel}>{t("settings.data.export.formatLabel")}</span>
          <div className={styles.options}>
            {FORMAT_ORDER.map((f) => (
              <label
                key={f}
                className={`${styles.option} ${format === f ? styles.optionChecked : ""}`}
              >
                <input
                  type="radio"
                  name="export-usage-format"
                  className={styles.optionRadio}
                  checked={format === f}
                  onChange={() => setFormat(f)}
                  disabled={busy}
                  aria-label={t(`settings.data.export.formats.${f}.label`)}
                />
                <div className={styles.optionBody}>
                  <div className={styles.optionLabel}>
                    {t(`settings.data.export.formats.${f}.label`)}
                    {f === "markdown" && (
                      <span className={styles.recommendBadge}>
                        {t("settings.data.export.recommended")}
                      </span>
                    )}
                  </div>
                  <div className={styles.optionHint}>
                    {t(`settings.data.export.formats.${f}.hint`, {
                      n: MARKDOWN_TOP_APPS,
                    })}
                  </div>
                </div>
              </label>
            ))}
          </div>
        </div>

        {/* ── 结果反馈 ── */}
        {donePath && (
          <div className={styles.doneBar}>
            <CheckCircle2 size={15} strokeWidth={2} className={styles.doneIcon} />
            <div className={styles.doneBody}>
              <span>{t("settings.data.export.doneTitle")}</span>
              <code className={styles.donePath}>{donePath}</code>
            </div>
            <button type="button" className={styles.revealBtn} onClick={revealDone}>
              {t("settings.data.export.reveal")}
            </button>
          </div>
        )}
        {error && (
          <p className={styles.errorBar}>{t("settings.data.export.error", { message: error })}</p>
        )}

        <div className={styles.actions}>
          <button
            type="button"
            className={`${styles.btn} ${styles.btnCancel}`}
            onClick={onClose}
            disabled={busy}
          >
            {donePath ? t("common.close") : t("common.cancel")}
          </button>
          <button
            type="button"
            className={`${styles.btn} ${styles.btnPrimary}`}
            onClick={runExport}
            disabled={!canExport}
            aria-busy={busy}
          >
            {busy && <Loader2 size={13} strokeWidth={2.25} className={styles.spinning} />}
            {busy ? t("settings.data.export.busy") : t("settings.data.export.confirm")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
