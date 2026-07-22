import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { Loader2, ScanEye, SlidersHorizontal, Wallet, X } from "lucide-react";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { Toggle } from "../../../components/FormControls/Toggle";
import { SimplePicker } from "../../../components/SimplePicker/SimplePicker";
import { useFocusTrap } from "../../../hooks/useFocusTrap";
import { useSettings } from "../../../state/settings";
import { api, type InsightStatus, type Settings } from "../../../api/hindsight";
import pageStyles from "../AISettings.module.css";
import styles from "./InsightTab.module.css";

type Scope = Settings["insightScope"];

/**
 * 云端截图洞察(行为层,docs/design/cloud-insight.md):
 * 开关+同意门 / 分析范围三档+重点应用 / 每日预算与用量 / 历史回填。
 * API 连接与模型名不在这里——那是「云端 API」(连接层)与「模型」(分配层)的事。
 */
export default function InsightTab() {
  const { t } = useTranslation();
  const { settings, update } = useSettings();
  const [status, setStatus] = useState<InsightStatus | null>(null);
  const [consentOpen, setConsentOpen] = useState(false);
  const [backfillEstimate, setBackfillEstimate] = useState<number | null>(null);
  const [backfillBusy, setBackfillBusy] = useState(false);

  const enabled = settings?.insightEnabled ?? false;
  const backfillRunning = status?.backfill?.running ?? false;

  const refreshStatus = useCallback(() => {
    api
      .insightStatus()
      .then(setStatus)
      .catch(() => setStatus(null));
  }, []);

  // 开着(或回填在跑)时轮询用量与进度;5s 与后端 tick 同量级
  useEffect(() => {
    if (!enabled) return;
    refreshStatus();
    const id = setInterval(refreshStatus, 5000);
    return () => clearInterval(id);
  }, [enabled, refreshStatus]);

  if (!settings) return null;

  const screenshotOn = settings.captureEnabled && settings.screenshotEnabled;
  const visionConfigured =
    settings.ai.visionModel.trim() !== "" &&
    (settings.ai.visionReuseText
      ? settings.ai.endpoint.trim() !== ""
      : settings.ai.visionEndpoint.trim() !== "");

  const onToggle = (next: boolean) => {
    if (!next) {
      update({ insightEnabled: false });
      return;
    }
    if (settings.insightConsentAcknowledged) {
      update({ insightEnabled: true });
    } else {
      setConsentOpen(true);
    }
  };

  const scopeOptions = (["focus", "recommended", "all"] as Scope[]).map((v) => ({
    value: v,
    label: t(`aiSettings.insight.scope.${v}`),
  }));

  const onBackfillClick = async () => {
    setBackfillBusy(true);
    try {
      const frames = await api.insightBackfillEstimate();
      setBackfillEstimate(frames);
    } finally {
      setBackfillBusy(false);
    }
  };

  const startBackfill = async () => {
    setBackfillEstimate(null);
    await api.insightBackfillStart();
    refreshStatus();
  };

  return (
    <div className={pageStyles.content}>
      <Section
        title={t("aiSettings.insight.sectionTitle")}
        icon={ScanEye}
        description={t("aiSettings.insight.sectionDesc")}
      >
        <Row
          label={t("aiSettings.insight.enableLabel")}
          description={
            screenshotOn
              ? t("aiSettings.insight.enableHint")
              : t("aiSettings.insight.needScreenshot")
          }
          disabled={!screenshotOn}
        >
          <Toggle
            checked={enabled}
            onChange={onToggle}
            ariaLabel={t("aiSettings.insight.enableLabel")}
          />
        </Row>
        {enabled ? (
          <div
            className={`${styles.depLine} ${visionConfigured ? "" : styles.depWarn}`}
          >
            {visionConfigured
              ? t("aiSettings.insight.visionReady", {
                  model: settings.ai.visionModel.trim(),
                })
              : t("aiSettings.insight.visionMissing")}
          </div>
        ) : null}
      </Section>

      {enabled ? (
        <>
          <Section
            title={t("aiSettings.insight.scopeTitle")}
            icon={SlidersHorizontal}
            description={t("aiSettings.insight.scopeDesc")}
          >
            <Row label={t("aiSettings.insight.scopeLabel")}>
              <SimplePicker<Scope>
                value={settings.insightScope}
                options={scopeOptions}
                onChange={(v) => update({ insightScope: v })}
              />
            </Row>
            <Row
              label={t("aiSettings.insight.focusAppsLabel")}
              description={t("aiSettings.insight.focusAppsHint")}
              block
            >
              <FocusAppsEditor
                apps={settings.insightFocusApps}
                onChange={(apps) => update({ insightFocusApps: apps })}
              />
            </Row>
            <div className={styles.usageLine}>
              {t(`aiSettings.insight.scopeExplain.${settings.insightScope}`)}
            </div>
          </Section>

          <Section
            title={t("aiSettings.insight.budgetTitle")}
            icon={Wallet}
            description={t("aiSettings.insight.budgetDesc")}
          >
            <Row label={t("aiSettings.insight.dailyCapLabel")}>
              <input
                type="number"
                className={styles.capInput}
                min={100}
                max={10000}
                value={settings.insightDailyFrameCap}
                onChange={(e) => {
                  const v = Number(e.target.value);
                  if (Number.isFinite(v)) {
                    update({ insightDailyFrameCap: v });
                  }
                }}
              />
            </Row>
            {status ? (
              <div className={styles.usageLine}>
                {t("aiSettings.insight.usageLine", {
                  done: status.todayDone,
                  cap: status.dailyCap,
                  pending: status.pending,
                })}
              </div>
            ) : null}
          </Section>

          <Section
            title={t("aiSettings.insight.backfillTitle")}
            icon={ScanEye}
            description={t("aiSettings.insight.backfillDesc")}
          >
            {backfillRunning && status?.backfill ? (
              <>
                <div className={styles.usageLine}>
                  {t("aiSettings.insight.backfillProgress", {
                    done: status.backfill.done,
                    total: status.backfill.total,
                  })}
                </div>
                <div className={styles.progressTrack}>
                  <div
                    className={styles.progressFill}
                    style={{
                      width: `${
                        status.backfill.total > 0
                          ? Math.min(
                              100,
                              (status.backfill.done / status.backfill.total) *
                                100,
                            )
                          : 0
                      }%`,
                    }}
                  />
                </div>
                <div style={{ marginTop: 10 }}>
                  <button
                    type="button"
                    className={styles.backfillBtn}
                    onClick={() => void api.insightBackfillCancel().then(refreshStatus)}
                  >
                    {t("common.cancel")}
                  </button>
                </div>
              </>
            ) : (
              <button
                type="button"
                className={styles.backfillBtn}
                disabled={backfillBusy || !visionConfigured}
                onClick={() => void onBackfillClick()}
              >
                {backfillBusy ? (
                  <Loader2 size={13} strokeWidth={2} className={styles.spinning} />
                ) : null}
                {t("aiSettings.insight.backfillButton")}
              </button>
            )}
          </Section>
        </>
      ) : null}

      <ConsentDialog
        open={consentOpen}
        onCancel={() => setConsentOpen(false)}
        onConfirm={() => {
          setConsentOpen(false);
          update({ insightEnabled: true, insightConsentAcknowledged: true });
        }}
      />
      <BackfillConfirmDialog
        frames={backfillEstimate}
        onCancel={() => setBackfillEstimate(null)}
        onConfirm={() => void startBackfill()}
      />
    </div>
  );
}

/** 重点应用 chips 编辑器:输入 process 名回车添加,点 × 删除。 */
function FocusAppsEditor({
  apps,
  onChange,
}: {
  apps: string[];
  onChange: (apps: string[]) => void;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState("");

  const add = () => {
    const v = draft.trim();
    if (!v || apps.includes(v)) {
      setDraft("");
      return;
    }
    onChange([...apps, v]);
    setDraft("");
  };

  return (
    <div>
      <div className={styles.chipsWrap}>
        {apps.map((a) => (
          <span key={a} className={styles.chip}>
            {a}
            <button
              type="button"
              className={styles.chipRemove}
              onClick={() => onChange(apps.filter((x) => x !== a))}
              aria-label={t("aiSettings.insight.removeApp", { app: a })}
            >
              <X size={11} strokeWidth={2.2} />
            </button>
          </span>
        ))}
      </div>
      <input
        type="text"
        className={styles.chipInput}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            add();
          }
        }}
        placeholder={t("aiSettings.insight.focusAppsPlaceholder")}
        spellCheck={false}
      />
    </div>
  );
}

/** 同意门:唯一入口。明示上传范围与目的地,确认落 insightConsentAcknowledged。 */
function ConsentDialog({
  open,
  onConfirm,
  onCancel,
}: {
  open: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  const ref = useRef<HTMLDivElement>(null);
  useFocusTrap(open, ref);
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onCancel]);

  if (!open) return null;
  return createPortal(
    <div className={styles.backdrop} onMouseDown={onCancel} role="presentation">
      {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions */}
      <div
        ref={ref}
        className={styles.dialog}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="insight-consent-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <h2 id="insight-consent-title" className={styles.dialogTitle}>
          {t("aiSettings.insight.consent.title")}
        </h2>
        <p className={styles.dialogBody}>{t("aiSettings.insight.consent.body")}</p>
        <ul className={styles.dialogList}>
          <li>{t("aiSettings.insight.consent.point1")}</li>
          <li>{t("aiSettings.insight.consent.point2")}</li>
          <li>{t("aiSettings.insight.consent.point3")}</li>
        </ul>
        <div className={styles.dialogActions}>
          <button type="button" className={styles.btn} onClick={onCancel}>
            {t("common.cancel")}
          </button>
          <button
            type="button"
            className={`${styles.btn} ${styles.btnConfirm}`}
            onClick={onConfirm}
            // eslint-disable-next-line jsx-a11y/no-autofocus
            autoFocus
          >
            {t("aiSettings.insight.consent.confirm")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

/** 回填确认:显示待分析帧数(费用取决于所配模型单价,由用户自行判断)。 */
function BackfillConfirmDialog({
  frames,
  onConfirm,
  onCancel,
}: {
  frames: number | null;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  const ref = useRef<HTMLDivElement>(null);
  const open = frames !== null;
  useFocusTrap(open, ref);
  if (!open) return null;
  return createPortal(
    <div className={styles.backdrop} onMouseDown={onCancel} role="presentation">
      {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions */}
      <div
        ref={ref}
        className={styles.dialog}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="insight-backfill-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <h2 id="insight-backfill-title" className={styles.dialogTitle}>
          {t("aiSettings.insight.backfillConfirm.title")}
        </h2>
        <p className={styles.dialogBody}>
          {frames === 0
            ? t("aiSettings.insight.backfillConfirm.empty")
            : t("aiSettings.insight.backfillConfirm.body", { frames })}
        </p>
        <div className={styles.dialogActions}>
          <button type="button" className={styles.btn} onClick={onCancel}>
            {t("common.cancel")}
          </button>
          {frames !== 0 ? (
            <button
              type="button"
              className={`${styles.btn} ${styles.btnConfirm}`}
              onClick={onConfirm}
              // eslint-disable-next-line jsx-a11y/no-autofocus
              autoFocus
            >
              {t("aiSettings.insight.backfillConfirm.confirm")}
            </button>
          ) : null}
        </div>
      </div>
    </div>,
    document.body,
  );
}
