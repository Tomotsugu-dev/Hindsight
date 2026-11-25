import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  AlertTriangle,
  ArrowUpFromLine,
  Download,
  FolderOpen,
  Globe,
  ScanText,
  Server,
  Trash2,
} from "lucide-react";
import { Section } from "../../../../components/FormLayout/Section";
import { Button } from "../../../../components/Button/Button";
import { ConfirmDialog } from "../../../../components/ConfirmDialog/ConfirmDialog";
import { RowMenu } from "../../../../components/RowMenu/RowMenu";
import { SimplePicker } from "../../../../components/SimplePicker/SimplePicker";
import { TextInput } from "../../../../components/TextInput/TextInput";
import {
  api,
  ENGINE_DOWNLOAD_EVENT,
  type DownloadSources,
  type EngineDownloadProgress,
  type EngineStatus,
} from "../../../../api/hindsight";
import { DEFAULT_DOWNLOAD_SOURCES } from "../../../../config/downloadSources";
import { useSettings } from "../../../../state/settings";
import { humanAccelLabel } from "../../../AISettings/shared/engineParams";
import { localizeAiError } from "../../../../lib/aiErrors";
import { logError, logWarn } from "../../../../lib/logger";
import type { EngineStatusState } from "./useEngineStatus";
import legacy from "../../../AISettings/AISettings.module.css";
import s from "./aiModels.module.css";

function message(e: unknown): string {
  return typeof e === "string" ? e : String(e);
}

/** What the computer needs to run models: the engine, text recognition, and where downloads come from. */
export function RuntimeSection({ engine }: { engine: EngineStatusState }) {
  const { t } = useTranslation();
  return (
    <Section
      title={t("settings.aiModels.runtime.title")}
      description={t("settings.aiModels.runtime.desc")}
    >
      {engine.status ? (
        <>
          <EngineRow status={engine.status} refresh={engine.refresh} />
          <OcrRow status={engine.status} refresh={engine.refresh} />
        </>
      ) : (
        <div className={s.empty}>{t("aiSettings.engine.loading")}</div>
      )}
      <DownloadSourceRow />
    </Section>
  );
}

interface RowProps {
  status: EngineStatus;
  refresh: () => Promise<void>;
}

/** The llama.cpp engine: its state, download or update, and freeing memory. Uninstall is in the menu. */
function EngineRow({ status, refresh }: RowProps) {
  const { t } = useTranslation();
  const [progress, setProgress] = useState<EngineDownloadProgress | null>(null);
  const [busy, setBusy] = useState(false);
  const [releasing, setReleasing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState(false);

  useEffect(() => {
    // 只管引擎这一段；文字识别组件的进度（stage = runtime）归下面那一行
    const p = listen<EngineDownloadProgress>(ENGINE_DOWNLOAD_EVENT, (ev) => {
      if (ev.payload.stage !== "runtime") setProgress(ev.payload);
    });
    return () => {
      void p.then((unlisten) => unlisten());
    };
  }, []);

  const installed = status.installed;
  const stale =
    installed && status.installedVersion !== null && status.installedVersion !== status.currentPin;
  const rt = status.runtime;

  const onDownload = async () => {
    setBusy(true);
    setError(null);
    setProgress(null);
    try {
      await api.downloadBinary(installed);
      await refresh();
      // 让「完成」停留一下再收起
      setTimeout(() => setProgress(null), 800);
    } catch (e) {
      setError(message(e));
      setProgress(null);
    } finally {
      setBusy(false);
    }
  };

  const onRelease = async () => {
    setReleasing(true);
    try {
      await api.stopEngine();
    } catch (e) {
      logWarn("engine.releaseVram", e);
    } finally {
      await refresh();
      setReleasing(false);
    }
  };

  const onUninstall = async () => {
    setConfirmingDelete(false);
    setBusy(true);
    setError(null);
    try {
      await api.deleteBinary();
      await refresh();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  };

  const version =
    (installed ? status.installedVersion : status.currentPin) ??
    t("aiSettings.engine.versionUnknown");
  const idleHint =
    rt.state === "running" && rt.idleSecondsRemaining != null
      ? formatIdleCountdown(rt.idleSecondsRemaining, t)
      : null;

  const pill = !installed
    ? { cls: s.pillWarn, text: t("aiSettings.engine.notInstalled"), title: undefined }
    : rt.state === "running"
      ? {
          cls: s.pillOk,
          text: t("settings.aiModels.engine.running"),
          title: t("aiSettings.engine.runtime.running", { port: rt.port }),
        }
      : rt.state === "starting"
        ? { cls: s.pillWarn, text: t("aiSettings.engine.runtime.starting"), title: undefined }
        : rt.state === "error"
          ? { cls: s.pillBad, text: t("aiSettings.engine.runtime.error"), title: undefined }
          : { cls: s.pillIdle, text: t("settings.aiModels.engine.stopped"), title: undefined };

  return (
    <div className={s.item}>
      <div className={s.row}>
        <span className={s.tile}>
          <Server aria-hidden />
        </span>
        <div className={s.main}>
          <span className={s.name}>{t("aiSettings.engine.sectionTitle")}</span>
          <span className={s.meta}>
            {t("settings.aiModels.engine.meta", {
              version,
              accel: humanAccelLabel(status.platformId, t),
              size: Math.round(status.estimatedBytes / 1024 / 1024),
            })}
          </span>
          {idleHint ? <span className={s.meta}>{idleHint}</span> : null}
        </div>
        <div className={s.side}>
          <span className={`${s.pill} ${pill.cls}`} title={pill.title}>
            {pill.text}
          </span>
          {!installed || stale ? (
            <Button
              variant="solid"
              tone="accent"
              icon={Download}
              loading={busy}
              onClick={() => void onDownload()}
            >
              {busy
                ? installed
                  ? t("aiSettings.engine.actions.updating")
                  : t("aiSettings.engine.actions.downloading")
                : installed
                  ? t("aiSettings.engine.actions.updateToLatest")
                  : t("aiSettings.engine.actions.downloadEngine")}
            </Button>
          ) : null}
          {rt.state === "running" ? (
            <Button
              variant="outline"
              icon={ArrowUpFromLine}
              loading={releasing}
              onClick={() => void onRelease()}
              title={t("aiSettings.engine.actions.releaseVramTooltip")}
            >
              {t("aiSettings.engine.actions.releaseVram")}
            </Button>
          ) : null}
          {installed ? (
            <RowMenu
              label={t("settings.aiModels.engine.more")}
              items={[
                {
                  label: t("settings.aiModels.engine.openDir"),
                  icon: FolderOpen,
                  onClick: () =>
                    void api.openEngineDir().catch((e) => logError("engine.openDir", e)),
                },
                {
                  label: t("settings.aiModels.engine.uninstall"),
                  icon: Trash2,
                  danger: true,
                  onClick: () => setConfirmingDelete(true),
                },
              ]}
            />
          ) : null}
        </div>
      </div>

      {status.platformId === "win-cpu-x64" ? (
        <div className={s.sub}>
          <div className={legacy.engineWarning}>
            <AlertTriangle size={14} strokeWidth={2.2} />
            <div className={legacy.engineWarningBody}>
              <strong>{t("aiSettings.engine.noCuda.headline")}</strong>
              <span>
                {t("aiSettings.engine.noCuda.prefix")}
                <button
                  type="button"
                  className={legacy.engineWarningLink}
                  onClick={() => void openUrl("https://developer.nvidia.com/cuda-downloads")}
                >
                  {t("aiSettings.engine.noCuda.linkText")}
                </button>
                {t("aiSettings.engine.noCuda.suffix")}
              </span>
            </div>
          </div>
        </div>
      ) : null}
      {/* key=stage：每段下载重新计最大值，不然第二段的 MB 数停在第一段 */}
      {progress ? (
        <div className={s.sub}>
          <EngineProgress key={progress.stage} progress={progress} />
        </div>
      ) : null}
      {error ? (
        <div className={s.sub}>
          <div className={legacy.engineError}>{error}</div>
        </div>
      ) : null}
      {rt.state === "error" && rt.error ? (
        <div className={s.sub}>
          <div className={legacy.engineError}>{localizeAiError(rt.error)}</div>
        </div>
      ) : null}

      <ConfirmDialog
        open={confirmingDelete}
        title={t("aiSettings.engine.uninstallConfirmTitle")}
        message={t("aiSettings.engine.uninstallConfirmMessage")}
        variant="danger"
        onConfirm={() => void onUninstall()}
        onCancel={() => setConfirmingDelete(false)}
      />
    </div>
  );
}

/**
 * The text recognition component for screen text search. Only Windows downloads it; macOS uses the
 * system's own recognition and shows no buttons.
 */
function OcrRow({ status, refresh }: RowProps) {
  const { t } = useTranslation();
  const [progress, setProgress] = useState<EngineDownloadProgress | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState(false);

  useEffect(() => {
    const p = listen<EngineDownloadProgress>(ENGINE_DOWNLOAD_EVENT, (ev) => {
      if (ev.payload.stage === "runtime") setProgress(ev.payload);
    });
    return () => {
      void p.then((unlisten) => unlisten());
    };
  }, []);

  const head = (meta: string) => (
    <>
      <span className={s.tile}>
        <ScanText aria-hidden />
      </span>
      <div className={s.main}>
        <span className={s.name}>{t("aiSettings.engine.ocr.name")}</span>
        <span className={s.meta}>{meta}</span>
      </div>
    </>
  );

  if (status.platformId.startsWith("macos")) {
    return (
      <div className={s.item}>
        <div className={s.row}>
          {head(t("aiSettings.engine.ocr.macNativeDesc"))}
          <div className={s.side}>
            <span className={`${s.pill} ${s.pillOk}`}>
              {t("aiSettings.engine.ocr.macNativeBadge")}
            </span>
          </div>
        </div>
      </div>
    );
  }

  const rt = status.embeddingRuntime;
  const installed = rt.installed;
  // 有版本文件却判未装：旧的 CPU 版还在，重新下载会换成显卡版
  const upgrade = !installed && !!rt.installedVersion;

  const onDownload = async () => {
    setBusy(true);
    setError(null);
    setProgress(null);
    try {
      await api.downloadOcrRuntime(installed);
      await refresh();
      setTimeout(() => setProgress(null), 800);
    } catch (e) {
      setError(message(e));
      setProgress(null);
    } finally {
      setBusy(false);
    }
  };

  const onUninstall = async () => {
    setConfirmingDelete(false);
    setBusy(true);
    setError(null);
    try {
      await api.deleteOcrRuntime();
      await refresh();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  };

  const size = t("aiSettings.engine.actions.approxSize", {
    size: Math.round(rt.estimatedBytes / 1024 / 1024),
  });

  return (
    <div className={s.item}>
      <div className={s.row}>
        {head(
          `${upgrade ? t("aiSettings.engine.ocr.upgradeHint") : t("aiSettings.engine.ocr.desc")} · ${size}`,
        )}
        <div className={s.side}>
          <span className={`${s.pill} ${installed ? s.pillOk : s.pillWarn}`}>
            {installed ? t("aiSettings.engine.installed") : t("aiSettings.engine.notInstalled")}
          </span>
          {installed ? (
            <RowMenu
              label={t("settings.aiModels.ocr.more")}
              items={[
                {
                  label: t("settings.aiModels.ocr.uninstall"),
                  icon: Trash2,
                  danger: true,
                  onClick: () => setConfirmingDelete(true),
                },
              ]}
            />
          ) : (
            <Button
              variant="outline"
              tone="accent"
              icon={Download}
              loading={busy}
              onClick={() => void onDownload()}
            >
              {busy
                ? t("aiSettings.engine.actions.downloading")
                : t("aiSettings.engine.ocr.download")}
            </Button>
          )}
        </div>
      </div>
      {progress ? (
        <div className={s.sub}>
          <EngineProgress key={progress.stage} progress={progress} />
        </div>
      ) : null}
      {error ? (
        <div className={s.sub}>
          <div className={legacy.engineError}>{error}</div>
        </div>
      ) : null}
      <ConfirmDialog
        open={confirmingDelete}
        title={t("aiSettings.engine.ocr.uninstallConfirmTitle")}
        message={t("aiSettings.engine.ocr.uninstallConfirmMessage")}
        variant="danger"
        onConfirm={() => void onUninstall()}
        onCancel={() => setConfirmingDelete(false)}
      />
    </div>
  );
}

function validAddress(value: string): boolean {
  try {
    const url = new URL(value.trim());
    return (
      ["http:", "https:"].includes(url.protocol) &&
      !!url.hostname &&
      !url.username &&
      !url.password &&
      !url.href.includes("?") &&
      !url.href.includes("#")
    );
  } catch {
    return false;
  }
}

/** Where the engine, the component and the models download from. A custom mirror opens three address fields. */
function DownloadSourceRow() {
  const { t } = useTranslation();
  const { settings, reload } = useSettings();
  const sources = settings?.downloadSources ?? DEFAULT_DOWNLOAD_SOURCES;
  const [huggingfaceUrl, setHuggingfaceUrl] = useState(sources.huggingfaceUrl);
  const [githubProxyUrl, setGithubProxyUrl] = useState(sources.githubProxyUrl);
  const [nugetUrl, setNugetUrl] = useState(sources.nugetUrl);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    setHuggingfaceUrl(sources.huggingfaceUrl);
    setGithubProxyUrl(sources.githubProxyUrl);
    setNugetUrl(sources.nugetUrl);
  }, [sources.huggingfaceUrl, sources.githubProxyUrl, sources.nugetUrl]);

  const save = async (patch: Partial<DownloadSources>) => {
    const next = { ...sources, ...patch };
    if (
      next.mode === "custom" &&
      (!validAddress(next.huggingfaceUrl) ||
        !validAddress(next.githubProxyUrl) ||
        !validAddress(next.nugetUrl))
    ) {
      setError(t("aiSettings.downloadSources.invalidUrl"));
      return;
    }
    setSaving(true);
    setError("");
    try {
      // 先存再让选择器显示新值，下载读到的才是存好的选择
      await api.updateSettings({ downloadSources: next });
      await reload();
    } catch (err) {
      logError("downloads.settings", err);
      setError(t("aiSettings.downloadSources.saveFailed"));
    } finally {
      setSaving(false);
    }
  };

  if (!settings) return null;

  const fields = [
    {
      key: "huggingface",
      value: huggingfaceUrl,
      set: setHuggingfaceUrl,
    },
    { key: "github", value: githubProxyUrl, set: setGithubProxyUrl },
    { key: "nuget", value: nugetUrl, set: setNugetUrl },
  ];

  return (
    <div className={s.item}>
      <div className={s.row}>
        <span className={s.tile}>
          <Globe aria-hidden />
        </span>
        <div className={s.main}>
          <span className={s.name}>{t("aiSettings.downloadSources.title")}</span>
          <span className={s.meta}>
            {sources.mode === "custom"
              ? t("settings.aiModels.source.customDesc")
              : t(`aiSettings.downloadSources.${sources.mode}Description`)}
          </span>
        </div>
        <div className={s.side}>
          <SimplePicker<DownloadSources["mode"]>
            value={sources.mode}
            disabled={saving}
            options={(["official", "mirror", "custom"] as const).map((mode) => ({
              value: mode,
              label: t(`aiSettings.downloadSources.modes.${mode}`),
            }))}
            onChange={(mode) => void save({ mode })}
          />
        </div>
      </div>
      {sources.mode === "custom" ? (
        <div className={s.panel}>
          {fields.map((f) => (
            <label key={f.key} className={s.field}>
              <span className={s.label}>{t(`aiSettings.downloadSources.${f.key}Label`)}</span>
              <TextInput
                type="url"
                value={f.value}
                disabled={saving}
                spellCheck={false}
                onChange={(event) => f.set(event.target.value)}
              />
              <span className={s.help}>{t(`aiSettings.downloadSources.${f.key}Help`)}</span>
            </label>
          ))}
          <div className={s.actions}>
            <Button
              variant="solid"
              tone="accent"
              disabled={saving}
              onClick={() =>
                void save({
                  huggingfaceUrl: huggingfaceUrl.trim(),
                  githubProxyUrl: githubProxyUrl.trim(),
                  nugetUrl: nugetUrl.trim(),
                })
              }
            >
              {t("aiSettings.downloadSources.save")}
            </Button>
          </div>
        </div>
      ) : null}
      {saving || error ? (
        <div className={s.sub}>
          {saving ? (
            <span className={s.meta} role="status">
              {t("aiSettings.downloadSources.saving")}
            </span>
          ) : null}
          {error ? (
            <div className={legacy.engineError} role="alert">
              {error}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/** The engine or component download: a bar while bytes come in, then one line for checking and unpacking. */
function EngineProgress({ progress }: { progress: EngineDownloadProgress }) {
  const { t } = useTranslation();
  // 只涨不跌：事件偶尔乱序，数字往回跳很难看
  const maxMbRef = useRef(0);
  const currentMb = Math.round(progress.downloaded / 1024 / 1024);
  if (currentMb > maxMbRef.current) maxMbRef.current = currentMb;
  const stage = t(
    progress.stage === "runtime"
      ? "aiSettings.engine.progress.stageRuntime"
      : "aiSettings.engine.progress.stageEngine",
  );
  if (progress.phase === "downloading") {
    return (
      <div className={s.progress}>
        <div className={s.track}>
          <div className={`${s.fill} ${s.fillUnknown}`} />
        </div>
        <span className={s.meta}>
          {stage} · {t("aiSettings.engine.progress.downloading", { size: maxMbRef.current })}
        </span>
      </div>
    );
  }
  const label =
    progress.phase === "verifying"
      ? t("aiSettings.engine.progress.verifying")
      : progress.phase === "extracting"
        ? t("aiSettings.engine.progress.extracting")
        : t("aiSettings.engine.progress.done");
  return (
    <span className={s.meta}>
      {stage} · {label}
    </span>
  );
}

/** Return "frees memory in X min Y s" or "in Y s". */
function formatIdleCountdown(
  seconds: number,
  t: (key: string, opts?: Record<string, unknown>) => string,
): string {
  const safe = Math.max(0, Math.floor(seconds));
  if (safe < 60) return t("aiSettings.engine.runtime.idleCountdownSec", { sec: safe });
  return t("aiSettings.engine.runtime.idleCountdownMin", {
    min: Math.floor(safe / 60),
    sec: safe % 60,
  });
}
