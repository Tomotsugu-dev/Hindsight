import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  AlertCircle,
  Cloud,
  Database,
  DatabaseBackup,
  DatabaseZap,
  FileDown,
  FolderOpen,
  HardDrive,
  ImageDown,
  PieChart,
  Trash2,
} from "lucide-react";
import { Button } from "../../../components/Button/Button";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { Slider } from "../../../components/FormControls/Slider";
import { PathField } from "../../../components/FormControls/PathField";
import { ConfirmDialog } from "../../../components/ConfirmDialog/ConfirmDialog";
import { RemoveDeviceDialog } from "../../../components/RemoveDeviceDialog/RemoveDeviceDialog";
import { ExportUsageDialog } from "../../../components/ExportUsageDialog/ExportUsageDialog";
import { useSettings } from "../../../state/settings";
import { api, type AuthState, type StorageInfo } from "../../../api/hindsight";
import { logError } from "../../../lib/logger";
import { canClearCloud } from "../../Devices/syncAccount";
import { formatBytes } from "../../../utils/formatBytes";

/** "db" = 从云端重建本机数据；"shots" = 清空截图；"remove" = 从云端移除本设备 */
type PurgeTarget = "db" | "shots" | "remove";

export default function DataTab() {
  const { t } = useTranslation();
  const { settings, update } = useSettings();
  const [storage, setStorage] = useState<StorageInfo | null>(null);
  /** 简单确认弹窗只用于 rebuild + shots；remove 走单独的 RemoveDeviceDialog */
  const [simpleConfirm, setSimpleConfirm] = useState<"db" | "shots" | null>(null);
  const [removeOpen, setRemoveOpen] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);
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

  // 「移除本设备」要先登录：打开页面时取一次登录状态，窗口聚焦时再刷
  const [auth, setAuth] = useState<AuthState | null>(null);
  useEffect(() => {
    const fetchAuth = () => {
      api
        .authStatus()
        .then(setAuth)
        .catch(() => setAuth(null));
    };
    fetchAuth();
    window.addEventListener("focus", fetchAuth);
    return () => window.removeEventListener("focus", fetchAuth);
  }, []);
  const canRemove = canClearCloud(auth);
  const onWebdav = auth?.backend === "webdav";

  if (!settings) return null;

  const total = storage ? storage.dbBytes + storage.memoryDbBytes + storage.screenshotsBytes : 0;

  const runSimple = async (which: "db" | "shots") => {
    setSimpleConfirm(null);
    setBusyTarget(which);
    try {
      if (which === "db") {
        await api.purgeLocalData();
      } else {
        await api.purgeScreenshots();
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

  const runRemove = async (keepLocal: boolean) => {
    setRemoveOpen(false);
    setBusyTarget("remove");
    try {
      const deleted = await api.purgeCloudData(keepLocal);
      window.alert(
        keepLocal
          ? t("settings.data.removeDeviceDialog.doneKeepLocal", {
              count: deleted,
            })
          : t("settings.data.removeDeviceDialog.doneAlsoClear", {
              count: deleted,
            }),
      );
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
      {/* ───── Section 1：存储用量 + AI 模型路径（蓝/中性）───── */}
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
                  db: formatBytes(storage.dbBytes),
                  mem: formatBytes(storage.memoryDbBytes),
                  shots: formatBytes(storage.screenshotsBytes),
                })
              : undefined
          }
          icon={PieChart}
        >
          <span
            style={{
              fontSize: "calc(var(--font-size-body) * var(--font-scale))",
              color: "var(--text-strong)",
              fontWeight: "var(--font-weight-600)",
            }}
          >
            {storage ? formatBytes(total) : "—"}
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
        <Row
          label={t("settings.data.export.rowLabel")}
          description={t("settings.data.export.rowDescription")}
          icon={FileDown}
        >
          <Button variant="outline" icon={FileDown} onClick={() => setExportOpen(true)}>
            {t("settings.data.export.rowButton")}
          </Button>
        </Row>
      </Section>

      {/* ───── Section 2：本地数据清理（蓝/中性，维护类，可恢复）───── */}
      <Section
        title={t("settings.data.cleanup.title")}
        description={t("settings.data.cleanup.description")}
        icon={DatabaseZap}
      >
        <Row
          label={t("settings.data.cleanup.purgeShotsLabel")}
          description={t("settings.data.cleanup.purgeShotsDescription")}
          icon={ImageDown}
        >
          <Button
            icon={FolderOpen}
            onClick={openShots}
            title={t("settings.data.cleanup.purgeShotsOpenTitle")}
          >
            {t("common.open")}
          </Button>
          <PurgeButton
            target="shots"
            variant="neutral"
            busyTarget={busyTarget}
            busyLabel={t("settings.data.cleanup.purgeShotsBusy")}
            idleLabel={t("common.clear")}
            onClick={() => setSimpleConfirm("shots")}
          />
        </Row>
        <Row
          label={t("settings.data.cleanup.purgeDbLabel")}
          description={t("settings.data.cleanup.purgeDbDescription")}
          icon={DatabaseBackup}
        >
          <Button
            icon={FolderOpen}
            onClick={revealDb}
            title={t("settings.data.cleanup.purgeDbOpenTitle")}
          >
            {t("common.open")}
          </Button>
          <PurgeButton
            target="db"
            variant="neutral"
            busyTarget={busyTarget}
            busyLabel={t("settings.data.cleanup.purgeDbBusy")}
            idleLabel={t("common.clear")}
            onClick={() => setSimpleConfirm("db")}
          />
        </Row>
      </Section>

      {/* ───── Section 3：危险区（红/警示，不可逆，影响所有设备）───── */}
      <Section
        title={t("settings.data.danger.title")}
        description={t("settings.data.danger.description")}
        icon={AlertCircle}
        tone="danger"
      >
        <Row
          label={t("settings.data.danger.removeDeviceLabel")}
          description={
            onWebdav
              ? t("settings.data.danger.removeDeviceNotOnWebdav")
              : canRemove
                ? t("settings.data.danger.removeDeviceDescription")
                : t("settings.data.danger.removeDeviceNeedsSignIn")
          }
          icon={Cloud}
          tone="danger"
        >
          <PurgeButton
            target="remove"
            variant="danger"
            busyTarget={busyTarget}
            busyLabel={t("settings.data.danger.removeDeviceBusy")}
            idleLabel={t("common.remove")}
            disabled={!canRemove}
            onClick={() => setRemoveOpen(true)}
          />
        </Row>
      </Section>

      {/* ───── 简单确认弹窗：rebuild + shots ───── */}
      <ConfirmDialog
        open={simpleConfirm !== null}
        title={
          simpleConfirm === "db"
            ? t("settings.data.purgeDialog.purgeDbTitle")
            : t("settings.data.purgeDialog.shotsTitle")
        }
        message={
          simpleConfirm === "db"
            ? t("settings.data.purgeDialog.purgeDbMessage")
            : t("settings.data.purgeDialog.shotsMessage")
        }
        confirmLabel={
          simpleConfirm === "db"
            ? t("settings.data.purgeDialog.purgeDbConfirm")
            : t("settings.data.purgeDialog.shotsConfirm")
        }
        cancelLabel={t("common.cancel")}
        variant="primary"
        onConfirm={() => simpleConfirm && runSimple(simpleConfirm)}
        onCancel={() => setSimpleConfirm(null)}
      />

      {/* ───── 复杂确认弹窗：从云端移除本设备（radio + 打字确认）───── */}
      <RemoveDeviceDialog
        open={removeOpen}
        onConfirm={runRemove}
        onCancel={() => setRemoveOpen(false)}
      />

      {/* ───── 导出使用数据（范围 / 粒度 / 格式配置）───── */}
      <ExportUsageDialog open={exportOpen} onClose={() => setExportOpen(false)} />
    </>
  );
}

/** 复用的删除按钮：支持 neutral（维护色）和 danger（红警示）两种 variant。
 *  自身正在跑 → spinner + busyLabel；别的按钮在跑 → 灰色 disabled；闲置 → 正常 trash 图标。
 *  三个按钮共享 busyTarget，互锁防并发。 */
function PurgeButton({
  target,
  variant,
  busyTarget,
  busyLabel,
  idleLabel,
  disabled = false,
  onClick,
}: {
  target: PurgeTarget;
  variant: "neutral" | "danger";
  busyTarget: PurgeTarget | null;
  busyLabel: string;
  idleLabel: string;
  disabled?: boolean;
  onClick: () => void;
}) {
  const isBusy = busyTarget === target;
  const isLocked = busyTarget !== null && !isBusy;
  return (
    <Button
      variant={variant === "danger" ? "soft" : "outline"}
      tone="danger"
      icon={Trash2}
      onClick={onClick}
      disabled={disabled || isLocked}
      loading={isBusy}
    >
      {isBusy ? busyLabel : idleLabel}
    </Button>
  );
}
