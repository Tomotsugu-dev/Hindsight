import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  AlertCircle,
  Cloud,
  Database,
  FolderOpen,
  HardDrive,
  ImageDown,
  Loader2,
  PieChart,
  Trash2,
} from "lucide-react";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { Slider } from "../../../components/FormControls/Slider";
import { PathField } from "../../../components/FormControls/PathField";
import { ConfirmDialog } from "../../../components/ConfirmDialog/ConfirmDialog";
import { useSettings } from "../../../state/settings";
import { api, type StorageInfo } from "../../../api/hindsight";
import { logError } from "../../../lib/logger";
import styles from "./DataTab.module.css";

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

type PurgeTarget = "db" | "shots" | "cloud";

export default function DataTab() {
  const { t } = useTranslation();
  const { settings, update } = useSettings();
  const [storage, setStorage] = useState<StorageInfo | null>(null);
  const [confirm, setConfirm] = useState<PurgeTarget | null>(null);
  // 哪一个 purge 操作正在跑：null = 三个按钮都空闲。busy 时**所有**三个按钮 disabled，
  // 避免用户连点 / 在一个 destructive op 跑到一半时触发另一个。busy 的那一个按钮显示
  // spinner + busy 文案；其它两个走 :disabled 的灰色路径。
  const [busyTarget, setBusyTarget] = useState<PurgeTarget | null>(null);

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
    setBusyTarget(target);
    try {
      if (target === "db") {
        await api.purgeActivities();
      } else if (target === "shots") {
        await api.purgeScreenshots();
      } else {
        // cloud
        const deleted = await api.purgeCloudData();
        window.alert(
          t("settings.data.purgeDialog.cloudDoneMessage", { count: deleted }),
        );
      }
      refreshStorage();
    } catch (e) {
      logError("data.clear", e);
      window.alert(
        t("settings.data.purgeDialog.error", {
          message: e instanceof Error ? e.message : String(e),
        }),
      );
    } finally {
      setBusyTarget(null);
    }
  };

  const revealDb = async () => {
    if (!storage) return;
    try {
      await revealItemInDir(storage.dbPath);
    } catch (e) {
      logError("data.openDbDir", e);
    }
  };

  const openShots = async () => {
    try {
      await api.openScreenshotsDir();
    } catch (e) {
      logError("data.openScreenshotDir", e);
    }
  };

  const updateAiModelsPath = (v: string) => {
    if (!settings) return;
    update({ ai: { ...settings.ai, modelsPath: v } });
  };

  const pickModelsDir = async () => {
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        defaultPath: settings.ai.modelsPath || undefined,
      });
      if (typeof picked === "string" && picked.length > 0) {
        updateAiModelsPath(picked);
      }
    } catch (e) {
      logError("data.openDirPicker", e);
    }
  };

  return (
    <>
      <Section
        title={t("settings.data.storage.title")}
        description={t("settings.data.storage.description")}
        icon={Database}
      >
        <Row
          label={t("settings.data.storage.retentionLabel")}
          description={t("settings.data.storage.retentionDescription")}
        >
          <Slider
            value={settings.retentionDays}
            onChange={(v) => update({ retentionDays: v })}
            min={1}
            max={120}
            step={1}
            suffix={t("common.units.days")}
          />
        </Row>
        <Row
          label={t("settings.data.storage.currentUsageLabel")}
          description={
            storage
              ? t("settings.data.storage.currentUsageDescription", {
                  db: fmtBytes(storage.dbBytes),
                  shots: fmtBytes(storage.screenshotsBytes),
                })
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
        <Row
          label={t("settings.data.storage.modelsPathLabel")}
          labelHint={t("settings.data.storage.modelsPathHint")}
          icon={HardDrive}
        >
          <PathField
            value={settings.ai.modelsPath}
            onChange={updateAiModelsPath}
            onPick={pickModelsDir}
          />
        </Row>
      </Section>

      <Section
        title={t("settings.data.manage.title")}
        description={t("settings.data.manage.description")}
        icon={AlertCircle}
        tone="danger"
      >
        <Row
          label={t("settings.data.manage.purgeDbLabel")}
          description={t("settings.data.manage.purgeDbDescription")}
          icon={Database}
          tone="danger"
        >
          <button
            type="button"
            className={styles.openBtn}
            onClick={revealDb}
            title={t("settings.data.manage.purgeDbOpenTitle")}
          >
            <FolderOpen size={14} strokeWidth={1.85} />
            {t("common.open")}
          </button>
          <PurgeButton
            target="db"
            busyTarget={busyTarget}
            busyLabel={t("settings.data.manage.purgeDbBusy")}
            onClick={() => setConfirm("db")}
          />
        </Row>
        <Row
          label={t("settings.data.manage.purgeShotsLabel")}
          description={t("settings.data.manage.purgeShotsDescription")}
          icon={ImageDown}
          tone="danger"
        >
          <button
            type="button"
            className={styles.openBtn}
            onClick={openShots}
            title={t("settings.data.manage.purgeShotsOpenTitle")}
          >
            <FolderOpen size={14} strokeWidth={1.85} />
            {t("common.open")}
          </button>
          <PurgeButton
            target="shots"
            busyTarget={busyTarget}
            busyLabel={t("settings.data.manage.purgeShotsBusy")}
            onClick={() => setConfirm("shots")}
          />
        </Row>
        <Row
          label={t("settings.data.manage.purgeCloudLabel")}
          description={t("settings.data.manage.purgeCloudDescription")}
          icon={Cloud}
          tone="danger"
        >
          <PurgeButton
            target="cloud"
            busyTarget={busyTarget}
            busyLabel={t("settings.data.manage.purgeCloudBusy")}
            onClick={() => setConfirm("cloud")}
          />
        </Row>
      </Section>

      <ConfirmDialog
        open={confirm !== null}
        title={
          confirm === "db"
            ? t("settings.data.purgeDialog.dbTitle")
            : confirm === "shots"
              ? t("settings.data.purgeDialog.shotsTitle")
              : t("settings.data.purgeDialog.cloudTitle")
        }
        message={
          confirm === "db"
            ? t("settings.data.purgeDialog.dbMessage")
            : confirm === "shots"
              ? t("settings.data.purgeDialog.shotsMessage")
              : t("settings.data.purgeDialog.cloudMessage")
        }
        confirmLabel={t("common.delete")}
        cancelLabel={t("common.cancel")}
        variant="danger"
        onConfirm={handleConfirm}
        onCancel={() => setConfirm(null)}
      />
    </>
  );
}

/** 复用的 danger 删除按钮：自身正在跑 → spinner + busyLabel + 高对比样式；
 *  别的按钮在跑 → 灰色 disabled；闲置 → 正常 trash 图标 + 「删除」字。
 *  三个按钮共享 busyTarget，互锁防并发。 */
function PurgeButton({
  target,
  busyTarget,
  busyLabel,
  onClick,
}: {
  target: PurgeTarget;
  busyTarget: PurgeTarget | null;
  busyLabel: string;
  onClick: () => void;
}) {
  const { t } = useTranslation();
  const isBusy = busyTarget === target;
  const isLocked = busyTarget !== null && !isBusy;
  return (
    <button
      type="button"
      className={`${styles.dangerBtn} ${isBusy ? styles.dangerBtnBusy : ""}`}
      onClick={onClick}
      disabled={isBusy || isLocked}
      aria-busy={isBusy}
    >
      {isBusy ? (
        <Loader2 size={13} strokeWidth={2.25} className={styles.spinning} />
      ) : (
        <Trash2 size={13} strokeWidth={2.25} />
      )}
      {isBusy ? busyLabel : t("common.delete")}
    </button>
  );
}
