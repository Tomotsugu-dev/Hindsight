import { useEffect, useState } from "react";
import { Section } from "../components/Section";
import { Row } from "../components/Row";
import { Slider } from "../components/Slider";
import { useSettings } from "../../../state/settings";
import { api, type StorageInfo } from "../../../api/hindsight";

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

export default function DataTab() {
  const { settings, update } = useSettings();
  const [storage, setStorage] = useState<StorageInfo | null>(null);

  useEffect(() => {
    const load = () => {
      api
        .getStorageInfo()
        .then(setStorage)
        .catch(() => setStorage(null));
    };
    load();
    const t = setInterval(load, 30_000);
    return () => clearInterval(t);
  }, []);

  if (!settings) return null;

  const total = storage ? storage.dbBytes + storage.screenshotsBytes : 0;

  return (
    <>
      <Section
        title="采集"
        description="窗口信息和截图的采集频率。间隔越短，记录越精细，磁盘占用越大。"
      >
        <Row label="采集间隔">
          <Slider
            value={settings.captureIntervalSeconds}
            onChange={(v) => update({ captureIntervalSeconds: v })}
            min={5}
            max={120}
            step={5}
            suffix="秒"
          />
        </Row>
      </Section>

      <Section title="存储" description="数据保留时长和当前占用情况。">
        <Row label="保留天数" description="超期数据将被自动清理。">
          <Slider
            value={settings.retentionDays}
            onChange={(v) => update({ retentionDays: v })}
            min={1}
            max={120}
            step={1}
            suffix="天"
          />
        </Row>
        <Row
          label="当前占用"
          description={
            storage
              ? `数据库 ${fmtBytes(storage.dbBytes)} · 截图 ${fmtBytes(storage.screenshotsBytes)}`
              : undefined
          }
        >
          <span style={{ fontSize: 13, color: "var(--text-strong)", fontWeight: 600 }}>
            {storage ? fmtBytes(total) : "—"}
          </span>
        </Row>
      </Section>
    </>
  );
}
