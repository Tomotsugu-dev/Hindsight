import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  AlertCircle,
  Database,
  FolderOpen,
  HardDrive,
  ImageDown,
  PieChart,
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

type PurgeTarget = "db" | "shots";

export default function DataTab() {
  const { t } = useTranslation();
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
      logError("data.clear", e);
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
          <button
            type="button"
            className={styles.dangerBtn}
            onClick={() => setConfirm("db")}
          >
            {t("common.delete")}
          </button>
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
          <button
            type="button"
            className={styles.dangerBtn}
            onClick={() => setConfirm("shots")}
          >
            {t("common.delete")}
          </button>
        </Row>
      </Section>

      <ConfirmDialog
        open={confirm !== null}
        title={
          confirm === "db"
            ? t("settings.data.purgeDialog.dbTitle")
            : t("settings.data.purgeDialog.shotsTitle")
        }
        message={
          confirm === "db"
            ? t("settings.data.purgeDialog.dbMessage")
            : t("settings.data.purgeDialog.shotsMessage")
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
