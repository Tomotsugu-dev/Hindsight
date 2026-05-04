import { useEffect, useState } from "react";
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
        title="采集"
        description="定时记录焦点窗口和当时的屏幕截图，作为之后回看的依据。"
        icon={Aperture}
      >
        <Row label="启用采集" description="关闭后停止记录窗口信息和截图。">
          <Toggle
            checked={settings.captureEnabled}
            onChange={(v) => update({ captureEnabled: v })}
          />
        </Row>
        <Row
          label="采集间隔"
          description="窗口信息和截图的采集频率。间隔越短，记录越精细，磁盘占用越大。"
          disabled={!settings.captureEnabled}
          labelHint={
            "截图触发时机：\n" +
            "• 切换应用 → 立即截图\n" +
            "• 浏览器切换 URL → 立即截图\n" +
            "• 同一窗口停留满设定的间隔 → 截图"
          }
        >
          <Slider
            value={settings.captureIntervalSeconds}
            onChange={(v) => update({ captureIntervalSeconds: v })}
            min={5}
            max={120}
            step={5}
            suffix="秒"
          />
        </Row>
        <Row label="截图保存路径" disabled={!settings.captureEnabled}>
          <PathField
            value={settings.screenshotPath}
            onChange={(v) => update({ screenshotPath: v })}
            onPick={pickScreenshotDir}
          />
        </Row>
        <Row
          label="数据保存路径"
          description="数据库与默认截图根目录所在位置。更改后需重启应用，旧数据需手动迁移。"
        >
          <PathField value={dataRoot} onPick={pickDataDir} readOnly />
        </Row>
      </Section>

      <Section
        title="工作时段"
        description="只在指定时段内采集，避免下班后还在记录。可设置多段。"
        icon={Clock}
      >
        <Row label="启用工作时段">
          <Toggle
            checked={settings.workHoursEnabled}
            onChange={(v) => update({ workHoursEnabled: v })}
          />
        </Row>
        <Row label="时间段" disabled={!settings.workHoursEnabled} block>
          <TimeRangeList
            ranges={settings.workRanges}
            onChange={(v) => update({ workRanges: v })}
          />
        </Row>
      </Section>

      <Section title="启动行为" icon={Rocket}>
        <Row label="开机自动启动" description="登录系统后由 Hindsight 自动运行。">
          <Toggle
            checked={settings.autoStart}
            onChange={(v) => update({ autoStart: v })}
          />
        </Row>
        <Row
          label="启动时显示主窗口"
          description="关闭则只在托盘待命，需要时手动唤起。"
          disabled={!settings.autoStart}
        >
          <Toggle
            checked={settings.showWindowOnAutoStart}
            onChange={(v) => update({ showWindowOnAutoStart: v })}
          />
        </Row>
        <Row
          label="关闭后最小化到右下角托盘"
          description={`点窗口${isMacOS ? "左上角" : "右上角"} X 时隐藏到系统托盘，采集与同步继续在后台运行。关闭则点 X 直接退出应用。`}
        >
          <Toggle
            checked={settings.minimizeToTray}
            onChange={(v) => update({ minimizeToTray: v })}
          />
        </Row>
      </Section>

      <ConfirmDialog
        open={pendingDataRoot !== null}
        title="切换数据保存路径？"
        message={`新路径：${pendingDataRoot ?? ""}\n\n保存后需要重启应用才会生效。\n\n旧目录的数据库与截图不会自动迁移——若想保留历史，请先手动把 ${dataRoot} 下的内容拷贝到新位置。`}
        confirmLabel="保存"
        cancelLabel="取消"
        variant="primary"
        onConfirm={confirmDataRoot}
        onCancel={() => setPendingDataRoot(null)}
      />
    </>
  );
}
