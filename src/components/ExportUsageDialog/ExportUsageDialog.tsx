import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { save } from "@tauri-apps/plugin-dialog";
import { downloadDir, join } from "@tauri-apps/api/path";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { CheckCircle2, Loader2 } from "lucide-react";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import { DateRangePicker } from "../DateRangePicker/DateRangePicker";
import { useDurationFormatter } from "../../utils/duration";
import { api } from "../../api/hindsight";
import {
  collectUsageData,
  fmtLocalDate,
  renderUsageExport,
  usageExportFilename,
  type UsageExportFormat,
} from "../../lib/usageExport";
import { buildUsageWorkbook } from "../../lib/usageXlsx";
import { logError } from "../../lib/logger";
import styles from "./ExportUsageDialog.module.css";

interface Props {
  open: boolean;
  onClose: () => void;
}

/** 快速范围键(交互定稿):点击把自然周期回填进两个日期框,手改日期即取消高亮。 */
type QuickKey = "today" | "week" | "month" | "year" | "all";

const QUICK_ORDER: QuickKey[] = ["today", "week", "month", "year", "all"];

/** 各格式的 save dialog 文件过滤器。 */
const SAVE_FILTERS: Record<UsageExportFormat, { name: string; extensions: string[] }> = {
  xlsx: { name: "Excel", extensions: ["xlsx"] },
  json: { name: "JSON", extensions: ["json"] },
  markdown: { name: "Markdown", extensions: ["md"] },
};

/** Markdown 排最前并作为默认：多数用户要的是"能直接看的报告"。xlsx 紧随其后
 *  （多 sheet 宽表，给要在表格软件里看/画图的人）；JSON 留给程序处理。 */
const FORMAT_ORDER: UsageExportFormat[] = ["markdown", "xlsx", "json"];

/** 范围跨度(含两端;"2026-07-18"~"2026-07-19" = 2)。round 抹掉夏令时 ±1h。 */
function spanDaysOf(start: string, end: string): number {
  const parse = (s: string): Date => {
    const [y, m, d] = s.split("-").map((v) => parseInt(v, 10));
    return new Date(y, m - 1, d);
  };
  return Math.round((parse(end).getTime() - parse(start).getTime()) / 86_400_000) + 1;
}

/** 快速键 → 自然周期范围(起 = 周期第一天,止 = 今天;"all" 由调用方先取最早日期)。 */
function quickRange(key: Exclude<QuickKey, "all">): { start: string; end: string } {
  const today = new Date();
  const end = fmtLocalDate(today);
  const start = new Date(today);
  if (key === "week") {
    const day = today.getDay(); // 0=周日;周口径 = 周一起
    start.setDate(start.getDate() - (day === 0 ? 6 : day - 1));
  } else if (key === "month") {
    start.setDate(1);
  } else if (key === "year") {
    start.setMonth(0, 1);
  }
  return { start: fmtLocalDate(start), end };
}

/**
 * 「导出使用数据」配置弹窗（设置 → 数据 → 存储）。
 * 日期范围（两个日期框常驻 + 今天/本周/本月/今年/全部快速填充）+ 统计粒度
 * （日 / 周 / 月）+ 文件格式（Markdown / Excel 表格 / JSON），确认后走
 * save dialog（默认下载目录）选路径 → 拉统计 → 写盘。只导统计数据，不含原始
 * 活动记录（后端也没有暴露原始记录查询）。
 */
export function ExportUsageDialog({ open, onClose }: Props) {
  const { t, i18n } = useTranslation();
  const fmtDuration = useDurationFormatter();

  const [start, setStart] = useState("");
  const [end, setEnd] = useState("");
  const [quick, setQuick] = useState<QuickKey | null>(null);
  const [format, setFormat] = useState<UsageExportFormat>("markdown");
  const [busy, setBusy] = useState(false);
  const [donePath, setDonePath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // 「全部」的起点(最早记录日):首次点击时查询并缓存;空库回退今天
  const earliestRef = useRef<string | null>(null);

  const todayStr = fmtLocalDate(new Date());

  // 每次打开重置：默认「本月」，清掉上次的结果 / 错误
  useEffect(() => {
    if (open) {
      const def = quickRange("month");
      setStart(def.start);
      setEnd(def.end);
      setQuick("month");
      setFormat("markdown");
      setBusy(false);
      setDonePath(null);
      setError(null);
    }
  }, [open]);

  const rangeValid = start.length > 0 && end.length > 0 && start <= end;
  // 粒度按范围跨度自动推导(勾选框已砍——"范围选今天却导出整月"这类
  // 许诺被粒度放大的组合从源头消灭):
  //   ≤7 天 → 每日;8~27 → +每周;28~92 → +每月;>92 → 仅 周+月
  //   (几百行的每日表可读性与 collect 成本双输;应用趋势矩阵以日期为列,
  //    92 天上限同时防住列爆炸;原始数据有明细 sheet 兜底)
  const spanDays = rangeValid ? spanDaysOf(start, end) : 0;
  const daily = spanDays > 0 && spanDays <= 92;
  const weekly = spanDays >= 8;
  const monthly = spanDays >= 28;
  const canExport = !busy && rangeValid;

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      // busy 中不许关：导出在途，关掉弹窗会丢掉成功 / 失败反馈
      if (e.key === "Escape" && !busy) onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, busy, onClose]);

  const dialogRef = useRef<HTMLDivElement>(null);
  useFocusTrap(open, dialogRef);

  if (!open) return null;

  const applyQuick = async (key: QuickKey) => {
    if (key === "all") {
      if (earliestRef.current === null) {
        try {
          earliestRef.current = (await api.earliestActivityDate()) ?? todayStr;
        } catch (e) {
          logError("data.exportUsage", e);
          earliestRef.current = todayStr;
        }
      }
      setStart(earliestRef.current);
      setEnd(todayStr);
    } else {
      const r = quickRange(key);
      setStart(r.start);
      setEnd(r.end);
    }
    setQuick(key);
  };

  const runExport = async () => {
    setError(null);
    setDonePath(null);

    // 先选保存位置再拉数据：用户取消 save dialog 时不做无谓查询。默认下载目录。
    let chosenPath: string | null = null;
    try {
      const filename = usageExportFilename({ rangeStart: start, rangeEnd: end }, format);
      let defaultPath = filename;
      try {
        defaultPath = await join(await downloadDir(), filename);
      } catch {
        // 拿不到下载目录(极少数环境)就退回裸文件名,由系统决定初始目录
      }
      chosenPath = await save({
        title: t("settings.data.export.dialogTitle"),
        defaultPath,
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
      const data = await collectUsageData({ start, end, daily, weekly, monthly }, labels);
      if (format === "xlsx") {
        const spec = buildUsageWorkbook(
          data,
          labels,
          t("settings.data.export.deviceAll"),
          todayStr,
          // 明细(原始记录)与统计同范围;设置页入口 = 全部设备
          { start: data.rangeStart, end: data.rangeEnd },
        );
        await api.exportUsageXlsx(chosenPath, spec);
      } else {
        await api.writeTextFile(chosenPath, renderUsageExport(data, format, labels));
      }
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

        {/* ── 日期范围:两个日期框常驻,快速按钮只负责回填 ── */}
        <div className={styles.field}>
          <span className={styles.fieldLabel}>{t("settings.data.export.rangeLabel")}</span>
          <div className={styles.rangeRow}>
            <DateRangePicker
              start={start}
              end={end}
              max={todayStr}
              disabled={busy}
              onChange={(s, e) => {
                setStart(s);
                setEnd(e);
                setQuick(null);
              }}
            />
          </div>
          <div className={styles.quickRow}>
            {QUICK_ORDER.map((key) => (
              <button
                key={key}
                type="button"
                className={`${styles.quickBtn} ${quick === key ? styles.quickBtnActive : ""}`}
                onClick={() => void applyQuick(key)}
                disabled={busy}
              >
                {t(`settings.data.export.quick.${key}`)}
              </button>
            ))}
          </div>
          {!rangeValid && (
            <p className={styles.fieldError}>{t("settings.data.export.invalidRange")}</p>
          )}
        </div>

        {/* 粒度按范围自动推导,只展示结果不给选择 */}
        {rangeValid && (
          <p className={styles.fieldHint}>
            {t("settings.data.export.included", {
              list: (
                [
                  ["daily", daily],
                  ["weekly", weekly],
                  ["monthly", monthly],
                ] as const
              )
                .filter(([, on]) => on)
                .map(([key]) => t(`settings.data.export.granularity.${key}`))
                .join(" · "),
            })}
          </p>
        )}

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
                    {t(`settings.data.export.formats.${f}.hint`)}
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
