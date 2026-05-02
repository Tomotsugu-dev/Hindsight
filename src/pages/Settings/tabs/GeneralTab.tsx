import { useState } from "react";
import { Section } from "../components/Section";
import { Row } from "../components/Row";
import { Toggle } from "../components/Toggle";
import { PathField } from "../components/PathField";
import { TimeRangeList, type TimeRange } from "../components/TimeRangeList";

export default function GeneralTab() {
  // 临时本地状态 — 后续接 tauri-plugin-store 持久化
  const [captureEnabled, setCaptureEnabled] = useState(true);
  const [screenshotPath, setScreenshotPath] = useState("~/Hindsight/screenshots");

  const [workHoursEnabled, setWorkHoursEnabled] = useState(false);
  const [workRanges, setWorkRanges] = useState<TimeRange[]>([]);

  const [autoStart, setAutoStart] = useState(false);
  const [showOnAutoStart, setShowOnAutoStart] = useState(false);

  return (
    <>
      <Section
        title="截图"
        description="定时把屏幕快照保存到本地，作为之后回看的依据。"
      >
        <Row label="启用截图" description="关闭后采集不再保存图像，仅记录窗口信息。">
          <Toggle checked={captureEnabled} onChange={setCaptureEnabled} />
        </Row>
        <Row label="保存路径" disabled={!captureEnabled}>
          <PathField value={screenshotPath} onChange={setScreenshotPath} />
        </Row>
      </Section>

      <Section
        title="工作时段"
        description="只在指定时段内采集，避免下班后还在记录。可设置多段。"
      >
        <Row label="启用工作时段">
          <Toggle checked={workHoursEnabled} onChange={setWorkHoursEnabled} />
        </Row>
        <Row label="时间段" disabled={!workHoursEnabled} block>
          <TimeRangeList ranges={workRanges} onChange={setWorkRanges} />
        </Row>
      </Section>

      <Section title="启动行为">
        <Row label="开机自动启动" description="登录系统后由 Hindsight 自动运行。">
          <Toggle checked={autoStart} onChange={setAutoStart} />
        </Row>
        <Row
          label="启动时显示主窗口"
          description="关闭则只在托盘待命，需要时手动唤起。"
          disabled={!autoStart}
        >
          <Toggle checked={showOnAutoStart} onChange={setShowOnAutoStart} />
        </Row>
      </Section>
    </>
  );
}
