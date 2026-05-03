import { open } from "@tauri-apps/plugin-dialog";
import { Aperture, Clock, Rocket } from "lucide-react";
import { Section } from "../components/Section";
import { Row } from "../components/Row";
import { Toggle } from "../components/Toggle";
import { PathField } from "../components/PathField";
import { TimeRangeList } from "../components/TimeRangeList";
import { useSettings } from "../../../state/settings";

export default function GeneralTab() {
  const { settings, update } = useSettings();
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
        <Row label="截图保存路径" disabled={!settings.captureEnabled}>
          <PathField
            value={settings.screenshotPath}
            onChange={(v) => update({ screenshotPath: v })}
            onPick={pickScreenshotDir}
          />
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
      </Section>
    </>
  );
}
