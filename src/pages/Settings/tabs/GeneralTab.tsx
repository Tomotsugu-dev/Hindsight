import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { platform } from "@tauri-apps/plugin-os";
import { Aperture, Clock, Languages, Loader2, Rocket } from "lucide-react";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { Toggle } from "../../../components/FormControls/Toggle";
import { PathField } from "../../../components/FormControls/PathField";
import { Slider } from "../../../components/FormControls/Slider";
import { TimeRangeList } from "../../../components/FormControls/TimeRangeList";
import { SimplePicker } from "../../../components/SimplePicker/SimplePicker";
import { ConfirmDialog } from "../../../components/ConfirmDialog/ConfirmDialog";
import { useSettings } from "../../../state/settings";
import { useLocale, LOCALE_OPTIONS } from "../../../i18n/useLocale";
import { api } from "../../../api/hindsight";
import { logError } from "../../../lib/logger";
import styles from "./GeneralTab.module.css";

export default function GeneralTab() {
  const { t } = useTranslation();
  const { settings, update } = useSettings();
  const [locale, setLocale] = useLocale();
  const [dataRoot, setDataRoot] = useState<string>("");
  const [pendingDataRoot, setPendingDataRoot] = useState<string | null>(null);
  // macOS 关闭按钮在窗口左上角；Win/Linux 在右上角。文案要根据平台变。
  const [isMacOS, setIsMacOS] = useState(false);
  // 历史截图回填：登记 + 立即识别，识别跑完积压才返回（可能数分钟）
  const [backfillBusy, setBackfillBusy] = useState(false);
  const [backfillMsg, setBackfillMsg] = useState("");

  useEffect(() => {
    api.getDataRoot().then(setDataRoot).catch(() => setDataRoot(""));
    setIsMacOS(platform() === "macos");
  }, []);

  if (!settings) return null;

  const pickScreenshotDir = async () => {
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        defaultPath: settings.screenshotPath || undefined,
      });
      if (typeof picked === "string" && picked.length > 0) {
        update({ screenshotPath: picked });
      }
    } catch (e) {
      logError("general.pickScreenshotDir", e);
    }
  };

  const pickDataDir = async () => {
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        defaultPath: dataRoot || undefined,
      });
      if (typeof picked === "string" && picked.length > 0 && picked !== dataRoot) {
        setPendingDataRoot(picked);
      }
    } catch (e) {
      logError("general.pickDataDir", e);
    }
  };

  const runBackfill = async () => {
    setBackfillBusy(true);
    setBackfillMsg("");
    // 识别期间每 3 秒查一次剩余帧数,实时显示进度;进入终态后停止覆盖
    let finished = false;
    const timer = setInterval(() => {
      void api
        .memoryPendingStats()
        .then((s) => {
          if (!finished && s.total > 0) {
            setBackfillMsg(
              t("settings.general.capture.backfillProgress", { n: s.total }),
            );
          }
        })
        .catch(() => {});
    }, 3000);
    try {
      const n = await api.memoryBackfill();
      setBackfillMsg(t("settings.general.capture.backfillRegistered", { n }));
      const rep = await api.memoryDigestNow();
      finished = true;
      setBackfillMsg(
        t("settings.general.capture.backfillDone", {
          ok: rep.processed,
          failed: rep.failed + rep.skippedMissingFile,
        }),
      );
    } catch (e) {
      finished = true;
      // 常驻批持锁时手动触发报"已在运行"——帧已登记,后台会消化,不算错误
      if (String(e).includes("已在运行")) {
        setBackfillMsg(t("settings.general.capture.backfillBackground"));
      } else {
        logError("general.backfill", e);
        setBackfillMsg(String(e));
      }
    } finally {
      clearInterval(timer);
      setBackfillBusy(false);
    }
  };

  const confirmDataRoot = async () => {
    if (!pendingDataRoot) return;
    try {
      await api.setDataRoot(pendingDataRoot);
      setDataRoot(pendingDataRoot);
    } catch (e) {
      logError("general.saveDataRoot", e);
    }
    setPendingDataRoot(null);
  };

  return (
    <>
      <Section title={t("settings.general.language.title")} icon={Languages}>
        <Row
          label={t("settings.general.language.label")}
          description={t("settings.general.language.description")}
        >
          <SimplePicker
            value={locale}
            options={LOCALE_OPTIONS}
            onChange={setLocale}
          />
        </Row>
      </Section>

      <Section
        title={t("settings.general.capture.title")}
        description={t("settings.general.capture.description")}
        icon={Aperture}
      >
        <Row
          label={t("settings.general.capture.enableLabel")}
          description={t("settings.general.capture.enableDescription")}
        >
          <Toggle
            checked={settings.captureEnabled}
            onChange={(v) => update({ captureEnabled: v })}
          />
        </Row>
        <Row
          label={t("settings.general.capture.screenshotEnableLabel")}
          description={t("settings.general.capture.screenshotEnableDescription")}
          disabled={!settings.captureEnabled}
        >
          <Toggle
            checked={settings.screenshotEnabled}
            onChange={(v) => update({ screenshotEnabled: v })}
          />
        </Row>
        <Row
          label={t("settings.general.capture.ocrResidentLabel")}
          description={t("settings.general.capture.ocrResidentDescription")}
          disabled={!settings.captureEnabled || !settings.screenshotEnabled}
        >
          <Toggle
            checked={settings.memoryOcrResident}
            onChange={(v) => update({ memoryOcrResident: v })}
          />
        </Row>
        <Row
          label={t("settings.general.capture.backfillLabel")}
          description={t("settings.general.capture.backfillDescription")}
        >
          <div className={styles.backfillWrap}>
            <button
              type="button"
              className={styles.backfillBtn}
              onClick={() => void runBackfill()}
              disabled={backfillBusy}
            >
              {backfillBusy ? (
                <>
                  <Loader2 size={13} strokeWidth={2.25} className={styles.spinning} />
                  {t("settings.general.capture.backfillRunning")}
                </>
              ) : (
                t("settings.general.capture.backfillButton")
              )}
            </button>
            {backfillMsg && <span className={styles.backfillMsg}>{backfillMsg}</span>}
          </div>
        </Row>
        <Row
          label={t("settings.general.capture.intervalLabel")}
          description={t("settings.general.capture.intervalDescription")}
          disabled={!settings.captureEnabled}
          labelHint={t("settings.general.capture.intervalHint")}
        >
          <Slider
            value={settings.captureIntervalSeconds}
            onChange={(v) => update({ captureIntervalSeconds: v })}
            min={5}
            max={120}
            step={5}
            suffix={t("common.units.seconds")}
          />
        </Row>
        <Row
          label={t("settings.general.capture.idleLabel")}
          description={t("settings.general.capture.idleDescription")}
          disabled={!settings.captureEnabled}
          labelHint={t("settings.general.capture.idleHint")}
        >
          <Slider
            value={Math.round(settings.idleThresholdSeconds / 60)}
            onChange={(v) => update({ idleThresholdSeconds: v * 60 })}
            min={0}
            max={30}
            step={1}
            suffix={t("common.units.minutes")}
          />
        </Row>
        <Row
          label={t("settings.general.capture.screenshotPathLabel")}
          disabled={!settings.captureEnabled}
        >
          <PathField
            value={settings.screenshotPath}
            onChange={(v) => update({ screenshotPath: v })}
            onPick={pickScreenshotDir}
          />
        </Row>
        <Row
          label={t("settings.general.capture.dataPathLabel")}
          labelHint={t("settings.general.capture.dataPathHint")}
        >
          <PathField value={dataRoot} onPick={pickDataDir} readOnly />
        </Row>
      </Section>

      <Section
        title={t("settings.general.workHours.title")}
        description={t("settings.general.workHours.description")}
        icon={Clock}
      >
        <Row label={t("settings.general.workHours.enableLabel")}>
          <Toggle
            checked={settings.workHoursEnabled}
            onChange={(v) => update({ workHoursEnabled: v })}
          />
        </Row>
        <Row
          label={t("settings.general.workHours.rangesLabel")}
          disabled={!settings.workHoursEnabled}
          block
        >
          <TimeRangeList
            ranges={settings.workRanges}
            onChange={(v) => update({ workRanges: v })}
          />
        </Row>
      </Section>

      <Section title={t("settings.general.startup.title")} icon={Rocket}>
        <Row
          label={t("settings.general.startup.autoStartLabel")}
          description={t("settings.general.startup.autoStartDescription")}
        >
          <Toggle
            checked={settings.autoStart}
            onChange={(v) => update({ autoStart: v })}
          />
        </Row>
        <Row
          label={t("settings.general.startup.showWindowLabel")}
          description={t("settings.general.startup.showWindowDescription")}
          disabled={!settings.autoStart}
        >
          <Toggle
            checked={settings.showWindowOnAutoStart}
            onChange={(v) => update({ showWindowOnAutoStart: v })}
          />
        </Row>
        <Row
          label={t("settings.general.startup.minimizeToTrayLabel")}
          description={
            isMacOS
              ? t("settings.general.startup.minimizeToTrayDescriptionMac")
              : t("settings.general.startup.minimizeToTrayDescriptionWin")
          }
        >
          <Toggle
            checked={settings.minimizeToTray}
            onChange={(v) => update({ minimizeToTray: v })}
          />
        </Row>
      </Section>

      <ConfirmDialog
        open={pendingDataRoot !== null}
        title={t("settings.general.dataRootDialog.title")}
        message={t("settings.general.dataRootDialog.message", {
          path: pendingDataRoot ?? "",
          oldPath: dataRoot,
        })}
        confirmLabel={t("common.save")}
        cancelLabel={t("common.cancel")}
        variant="primary"
        onConfirm={confirmDataRoot}
        onCancel={() => setPendingDataRoot(null)}
      />
    </>
  );
}
