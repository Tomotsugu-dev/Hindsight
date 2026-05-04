import { useEffect, useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  AlertCircle,
  Database,
  FolderOpen,
  ImageDown,
  PieChart,
} from "lucide-react";
import { Section } from "../components/Section";
import { Row } from "../components/Row";
import { Slider } from "../components/Slider";
import { ConfirmDialog } from "../../../components/ConfirmDialog/ConfirmDialog";
import { useSettings } from "../../../state/settings";
import { api, type StorageInfo } from "../../../api/hindsight";
import styles from "./DataTab.module.css";

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

type PurgeTarget = "db" | "shots";

export default function DataTab() {
  const { settings, update } = useSettings();
  const [storage, setStorage] = useState<StorageInfo | null>(null);
  const [confirm, setConfirm] = useState<PurgeTarget | null>(null);

  const refreshStorage = () => {
    api
      .getStorageInfo()
      .then(setStorage)
      .catch(() => setStorage(null));
  };

  useEffect(() => {
    refreshStorage();
    const t = setInterval(refreshStorage, 30_000);
    return () => clearInterval(t);
  }, []);

  if (!settings) return null;

  const total = storage ? storage.dbBytes + storage.screenshotsBytes : 0;

  const handleConfirm = async () => {
    const target = confirm;
    setConfirm(null);
    if (!target) return;
    try {
      if (target === "db") await api.purgeActivities();
      else await api.purgeScreenshots();
      refreshStorage();
    } catch (e) {
      console.error("清除失败:", e);
    }
  };

  const revealDb = async () => {
    if (!storage) return;
    try {
      await revealItemInDir(storage.dbPath);
    } catch (e) {
      console.error("打开数据库目录失败:", e);
    }
  };

  const openShots = async () => {
    try {
      await api.openScreenshotsDir();
    } catch (e) {
      console.error("打开截图目录失败:", e);
    }
  };

  return (
    <>
      <Section
        title="存储"
        description="截图保留时长和当前占用情况；数据库永不自动清理。"
        icon={Database}
      >
        <Row
          label="截图保留天数"
          description="到期的截图文件将被自动删除（活动记录本身保留）。"
        >
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
          icon={PieChart}
        >
          <span
            style={{ fontSize: 13, color: "var(--text-strong)", fontWeight: 600 }}
          >
            {storage ? fmtBytes(total) : "—"}
          </span>
        </Row>
      </Section>

      <Section
        title="数据管理"
        description="清理不需要的数据，释放磁盘空间。"
        icon={AlertCircle}
        tone="danger"
      >
        <Row
          label="清空数据库"
          description="删除所有窗口活动记录，保留分类和设置。"
          icon={Database}
          tone="danger"
        >
          <button
            type="button"
            className={styles.openBtn}
            onClick={revealDb}
            title="在文件管理器中显示数据库"
          >
            <FolderOpen size={14} strokeWidth={1.85} />
            打开
          </button>
          <button
            type="button"
            className={styles.dangerBtn}
            onClick={() => setConfirm("db")}
          >
            删除
          </button>
        </Row>
        <Row
          label="清空截图"
          description="删除已保存的全部截图文件。"
          icon={ImageDown}
          tone="danger"
        >
          <button
            type="button"
            className={styles.openBtn}
            onClick={openShots}
            title="打开截图保存文件夹"
          >
            <FolderOpen size={14} strokeWidth={1.85} />
            打开
          </button>
          <button
            type="button"
            className={styles.dangerBtn}
            onClick={() => setConfirm("shots")}
          >
            删除
          </button>
        </Row>
      </Section>

      <ConfirmDialog
        open={confirm !== null}
        title={confirm === "db" ? "清空数据库？" : "清空截图？"}
        message={
          confirm === "db"
            ? "将删除所有窗口活动记录。分类和设置不会受影响。此操作无法撤销。"
            : "将删除已保存的全部截图文件。数据库中对应的截图引用也会一并清除。此操作无法撤销。"
        }
        confirmLabel="删除"
        cancelLabel="取消"
        variant="danger"
        onConfirm={handleConfirm}
        onCancel={() => setConfirm(null)}
      />
    </>
  );
}
