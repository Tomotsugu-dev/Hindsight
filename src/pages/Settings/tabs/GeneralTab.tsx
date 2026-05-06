import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { platform } from "@tauri-apps/plugin-os";
import { Aperture, Clock, Rocket } from "lucide-react";
import { Section } from "../components/Section";
import { Row } from "../components/Row";
import { Toggle } from "../components/Toggle";
import { PathField } from "../components/PathField";
import { Slider } from "../components/Slider";
import { TimeRangeList } from "../components/TimeRangeList";
import { ConfirmDialog } from "../../../components/ConfirmDialog/ConfirmDialog";
import { useSettings } from "../../../state/settings";
import { api } from "../../../api/hindsight";

export default function GeneralTab() {
  const { t } = useTranslation();
  const { settings, update } = useSettings();
  const [dataRoot, setDataRoot] = useState<string>("");
  const [pendingDataRoot, setPendingDataRoot] = useState<string | null>(null);
  // macOS 关闭按钮在窗口左上角；Win/Linux 在右上角。文案要根据平台变。
  const [isMacOS, setIsMacOS] = useState(false);

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
      console.error("打开目录选择失败:", e);
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
      console.error("打开目录选择失败:", e);
    }
  };

  const confirmDataRoot = async () => {
    if (!pendingDataRoot) return;
    try {
      await api.setDataRoot(pendingDataRoot);
      setDataRoot(pendingDataRoot);
    } catch (e) {
      console.error("保存数据路径失败:", e);
    }
    setPendingDataRoot(null);
  };

  return (
    <>
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
