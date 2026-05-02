import { useState } from "react";
import { Section } from "../components/Section";
import { Row } from "../components/Row";
import { Slider } from "../components/Slider";

export default function DataTab() {
  const [intervalSec, setIntervalSec] = useState(20);
  const [retentionDays, setRetentionDays] = useState(7);

  return (
    <>
      <Section
        title="采集"
        description="窗口信息和截图的采集频率。间隔越短，记录越精细，磁盘占用越大。"
      >
        <Row label="采集间隔">
          <Slider
            value={intervalSec}
            onChange={setIntervalSec}
            min={5}
            max={120}
            step={5}
            suffix="秒"
          />
        </Row>
      </Section>

      <Section
        title="存储"
        description="数据保留时长和当前占用情况。"
      >
        <Row label="保留天数" description="超期数据将被自动清理。">
          <Slider
            value={retentionDays}
            onChange={setRetentionDays}
            min={1}
            max={120}
            step={1}
            suffix="天"
          />
        </Row>
        <Row label="当前占用" description="截图与数据库总大小（待接入）。">
          <span style={{ fontSize: 13, color: "var(--text-muted)" }}>—</span>
        </Row>
      </Section>
    </>
  );
}
