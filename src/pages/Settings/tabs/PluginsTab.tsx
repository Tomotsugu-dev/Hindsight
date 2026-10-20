import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { FolderOpen, Package, Puzzle, RefreshCw, ShieldAlert, Store } from "lucide-react";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { Toggle } from "../../../components/FormControls/Toggle";
import { SimplePicker } from "../../../components/SimplePicker/SimplePicker";
import { ConfirmDialog } from "../../../components/ConfirmDialog/ConfirmDialog";
import { EmptyHint } from "../../../components/EmptyHint/EmptyHint";
import {
  pluginsApi,
  type InstalledPlugin,
  type Market,
  type OnlinePlugin,
  type OnlineVersion,
  type OpensIn,
} from "../../../api/plugins";
import { markPluginsSeen } from "../../../state/pluginsSeen";
import { usePluginIcons } from "../../../state/pluginIcons";
import { logError } from "../../../lib/logger";
import { HIGH_RISK, permissionKey } from "./pluginPermissions";
import styles from "./PluginsTab.module.css";

/** Where a plugin comes from. Third-party plugins and zips get a warning in the install dialog */
type Source = "official" | "thirdParty" | "local";

/** What the install dialog shows, and how to install once the user agrees. */
interface InstallTarget {
  name: string;
  version: string;
  source: Source;
  /** From the online list. `null` for a zip */
  author: string | null;
  permissions: string[];
  storageMb: number | null;
  /** `null` when the online list does not say */
  runsInBackground: boolean | null;
  run: () => Promise<void>;
}

type OpensInChoice = "main" | "window";

/** A plugin whose manifest leaves out opens_in is not in the sidebar either, and opens in its own
 * window from here */
const toChoice = (opensIn: OpensIn): OpensInChoice => (opensIn === "main" ? "main" : "window");

/** The install dialog for one version in the online list. */
const onlineTarget = (plugin: OnlinePlugin, v: OnlineVersion): InstallTarget => ({
  name: plugin.name,
  version: v.version,
  source: plugin.official ? "official" : "thirdParty",
  author: plugin.author,
  permissions: v.permissions,
  storageMb: v.storage_mb,
  runsInBackground: null,
  run: () => pluginsApi.installOnline(plugin.id, v.version, v.permissions, v.storage_mb),
});

/** Settings → Plugins: installed plugins, the plugin market, and zips installed by hand.
 * Opening this tab is the only thing that fetches the online list. */
export default function PluginsTab() {
  const { t } = useTranslation();
  const icons = usePluginIcons();
  const [market, setMarket] = useState<Market | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  /** The plugin or zip with an action running; other actions wait for it */
  const [busy, setBusy] = useState<string | null>(null);
  const [installing, setInstalling] = useState<InstallTarget | null>(null);
  /** The checkbox in the install dialog. Install is allowed only when it is checked */
  const [agreed, setAgreed] = useState(false);
  const [uninstalling, setUninstalling] = useState<InstalledPlugin | null>(null);
  const [deleteData, setDeleteData] = useState(false);

  useEffect(() => markPluginsSeen(), []);

  const load = useCallback(() => {
    pluginsApi
      .openMarket()
      .then((m) => {
        setMarket(m);
        setLoadError(null);
      })
      .catch((e) => {
        logError("plugins.openMarket", e);
        setLoadError(String(e));
      });
  }, []);
  useEffect(load, [load]);

  /** Runs one action at a time; reloads the market when `reload` is set. */
  const act = async (key: string, action: () => Promise<void>, reload: boolean) => {
    setBusy(key);
    setActionError(null);
    try {
      await action();
      if (reload) load();
    } catch (e) {
      logError("plugins.action", e);
      setActionError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const updateInstalled = (id: string, change: (p: InstalledPlugin) => InstalledPlugin) =>
    setMarket((m) =>
      m ? { ...m, installed: m.installed.map((p) => (p.id === id ? change(p) : p)) } : m,
    );

  const setEnabled = (plugin: InstalledPlugin, enabled: boolean) =>
    act(
      plugin.id,
      async () => {
        await pluginsApi.setEnabled(plugin.id, enabled);
        updateInstalled(plugin.id, (p) => ({ ...p, enabled }));
      },
      false,
    );

  const setOpensIn = (plugin: InstalledPlugin, opensIn: OpensIn) =>
    act(
      plugin.id,
      async () => {
        await pluginsApi.setOpensIn(plugin.id, opensIn);
        updateInstalled(plugin.id, (p) => ({ ...p, ui: { ...p.ui, opensIn } }));
      },
      false,
    );

  const showWindow = (plugin: InstalledPlugin) => {
    pluginsApi.openWindow(plugin.id).catch((e) => {
      logError("plugins.openWindow", e);
      setActionError(String(e));
    });
  };

  const openInstall = (target: InstallTarget) => {
    setAgreed(false);
    setInstalling(target);
  };

  const confirmInstall = () => {
    const target = installing;
    setInstalling(null);
    if (target) void act(`install:${target.name}`, target.run, true);
  };

  const confirmUninstall = () => {
    const plugin = uninstalling;
    setUninstalling(null);
    if (plugin) void act(plugin.id, () => pluginsApi.uninstall(plugin.id, deleteData), true);
  };

  const opensInOptions = [
    { value: "main" as const, label: t("plugins.opensIn.main") },
    { value: "window" as const, label: t("plugins.opensIn.window") },
  ];

  if (!market) {
    return (
      <p className={styles.hint}>
        {loadError ? t("plugins.loadFailed", { error: loadError }) : t("plugins.loading")}
      </p>
    );
  }

  return (
    <>
      {actionError ? <p className={styles.error}>{actionError}</p> : null}

      <Section title={t("plugins.installed.title")} icon={Puzzle}>
        {market.installed.length === 0 ? (
          <EmptyHint message={t("plugins.installed.empty")} />
        ) : (
          market.installed.map((plugin) => {
            const onlinePlugin = market.installedOnline.find((o) => o.id === plugin.id);
            const online = onlinePlugin?.versions ?? [];
            // Versions come newest first, so the first update is the newest
            const update = online.find((v) => v.kind === "update");
            const versionOptions = online.map((v) => ({
              value: v.version,
              label:
                v.kind === "same"
                  ? t("plugins.installed.current", { version: v.version })
                  : `v${v.version}`,
            }));
            // Installed from a zip with a version the online list does not have
            if (online.length > 0 && !online.some((v) => v.kind === "same")) {
              versionOptions.unshift({
                value: plugin.version,
                label: t("plugins.installed.current", { version: plugin.version }),
              });
            }
            const working = busy !== null;
            return (
              <Row
                key={plugin.id}
                label={`${plugin.name}  v${plugin.version}`}
                description={
                  plugin.runsInBackground ? t("plugins.installed.background") : undefined
                }
                icon={Puzzle}
                iconSrc={icons[plugin.id]}
                block
                actions={
                  <Toggle
                    checked={plugin.enabled}
                    disabled={working}
                    ariaLabel={t("plugins.installed.enabled")}
                    onChange={(next) => void setEnabled(plugin, next)}
                  />
                }
              >
                <div className={styles.controls}>
                  {plugin.ui.hasPage ? (
                    <>
                      <SimplePicker
                        value={toChoice(plugin.ui.opensIn)}
                        options={opensInOptions}
                        disabled={working}
                        onChange={(choice) => void setOpensIn(plugin, choice)}
                      />
                      {/* A plugin in the sidebar is opened from there */}
                      {toChoice(plugin.ui.opensIn) === "window" ? (
                        <button
                          type="button"
                          className={styles.btn}
                          disabled={!plugin.enabled}
                          onClick={() => showWindow(plugin)}
                        >
                          {t("plugins.installed.show")}
                        </button>
                      ) : null}
                    </>
                  ) : null}
                  {versionOptions.length > 0 ? (
                    <SimplePicker
                      value={plugin.version}
                      options={versionOptions}
                      disabled={working}
                      onChange={(version) => {
                        const v = online.find((o) => o.version === version);
                        if (onlinePlugin && v && v.kind !== "same") {
                          openInstall(onlineTarget(onlinePlugin, v));
                        }
                      }}
                    />
                  ) : null}
                  {onlinePlugin && update ? (
                    <button
                      type="button"
                      className={styles.btnPrimary}
                      disabled={working}
                      onClick={() => openInstall(onlineTarget(onlinePlugin, update))}
                    >
                      {t("plugins.installed.update", { version: update.version })}
                    </button>
                  ) : null}
                  <button
                    type="button"
                    className={styles.btnDanger}
                    disabled={working}
                    onClick={() => {
                      setDeleteData(false);
                      setUninstalling(plugin);
                    }}
                  >
                    {busy === plugin.id ? t("plugins.working") : t("plugins.installed.uninstall")}
                  </button>
                </div>
              </Row>
            );
          })
        )}
      </Section>

      <Section title={t("plugins.market.title")} icon={Store}>
        {market.onlineError ? (
          <p className={styles.error}>
            {t("plugins.market.offline", { error: market.onlineError })}
          </p>
        ) : market.downloadable.length === 0 ? (
          <EmptyHint message={t("plugins.market.empty")} />
        ) : (
          <div className={styles.marketGrid}>
            {market.downloadable.map((plugin) => (
              <MarketCard
                key={plugin.id}
                plugin={plugin}
                working={busy === `install:${plugin.name}`}
                disabled={busy !== null}
                onInstall={() => openInstall(onlineTarget(plugin, plugin.versions[0]))}
              />
            ))}
          </div>
        )}
      </Section>

      <Section
        title={t("plugins.local.title")}
        description={t("plugins.local.description")}
        icon={Package}
        collapsible
        headerAction={
          <span className={styles.controls}>
            <button
              type="button"
              className={styles.btn}
              onClick={() =>
                pluginsApi.openZipsFolder().catch((e) => {
                  logError("plugins.openZipsFolder", e);
                  setActionError(String(e));
                })
              }
            >
              <FolderOpen size={14} strokeWidth={1.85} />
              {t("plugins.local.openFolder")}
            </button>
            <button type="button" className={styles.btn} onClick={load}>
              <RefreshCw size={14} strokeWidth={1.85} />
              {t("plugins.local.refresh")}
            </button>
          </span>
        }
      >
        {market.localZips.length === 0 ? (
          <EmptyHint message={t("plugins.local.empty")} />
        ) : (
          market.localZips.map((zip) => {
            const m = zip.manifest;
            const key = `install:${m?.name ?? zip.fileName}`;
            return (
              <Row
                key={zip.fileName}
                label={m ? `${m.name}  v${m.version}` : zip.fileName}
                description={zip.error ?? zip.fileName}
                icon={Puzzle}
              >
                <button
                  type="button"
                  className={styles.btnPrimary}
                  disabled={busy !== null || !m || zip.error !== null}
                  onClick={() =>
                    m &&
                    openInstall({
                      name: m.name,
                      version: m.version,
                      source: "local",
                      author: null,
                      permissions: m.permissions,
                      storageMb: m.storageMb,
                      runsInBackground: m.runsInBackground,
                      run: () => pluginsApi.install(zip.fileName, m.permissions, m.storageMb),
                    })
                  }
                >
                  {busy === key ? t("plugins.working") : t("plugins.market.install")}
                </button>
              </Row>
            );
          })
        )}
      </Section>

      <ConfirmDialog
        open={installing !== null}
        title={
          installing?.source === "thirdParty"
            ? t("plugins.install.trustTitle", { author: installing.author })
            : installing
              ? t("plugins.install.title", { name: installing.name })
              : ""
        }
        centerTitle
        notice={installing ? <InstallWarning target={installing} /> : null}
        message={
          installing ? (
            <InstallDetails target={installing} agreed={agreed} onAgree={setAgreed} />
          ) : null
        }
        confirmLabel={t("plugins.install.confirm")}
        confirmDisabled={!agreed}
        onConfirm={confirmInstall}
        onCancel={() => setInstalling(null)}
      />

      <ConfirmDialog
        open={uninstalling !== null}
        variant="danger"
        title={uninstalling ? t("plugins.uninstall.title", { name: uninstalling.name }) : ""}
        message={
          <label className={styles.check}>
            <input
              type="checkbox"
              checked={deleteData}
              onChange={(e) => setDeleteData(e.target.checked)}
            />
            {t("plugins.uninstall.deleteData")}
          </label>
        }
        confirmLabel={t("plugins.installed.uninstall")}
        onConfirm={confirmUninstall}
        onCancel={() => setUninstalling(null)}
      />
    </>
  );
}

/** One plugin in the market: its icon, name, latest version, author and description. */
function MarketCard({
  plugin,
  working,
  disabled,
  onInstall,
}: {
  plugin: OnlinePlugin;
  /** This plugin is being installed */
  working: boolean;
  disabled: boolean;
  onInstall: () => void;
}) {
  const { t } = useTranslation();
  const iconSrc = plugin.icon_url ?? null;
  // Kept per URL, so a new icon is tried again
  const [brokenSrc, setBrokenSrc] = useState<string | null>(null);
  const showImage = iconSrc !== null && iconSrc !== brokenSrc;
  return (
    <div className={styles.marketCard}>
      <div className={styles.marketHead}>
        <span className={`${styles.marketIcon} ${showImage ? "" : styles.marketIconFallback}`}>
          {showImage ? (
            // onError falls back to the puzzle icon when the image is missing
            // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions
            <img src={iconSrc} alt="" onError={() => setBrokenSrc(iconSrc)} />
          ) : (
            <Puzzle size={18} strokeWidth={1.85} />
          )}
        </span>
        <div className={styles.marketTitle}>
          <span className={styles.marketName}>{plugin.name}</span>
          <span className={styles.marketMeta}>
            v{plugin.versions[0].version} · {plugin.author}
          </span>
        </div>
      </div>
      <p className={styles.marketDesc}>{plugin.description}</p>
      <div className={styles.marketFoot}>
        <button type="button" className={styles.btnPrimary} disabled={disabled} onClick={onInstall}>
          {working ? t("plugins.working") : t("plugins.market.install")}
        </button>
      </div>
    </div>
  );
}

/** The warning for a third-party plugin or a zip. It sits outside the scrolling details, so it is
 * still in view when the user scrolls down to agree. Nothing for an official plugin. */
function InstallWarning({ target }: { target: InstallTarget }) {
  const { t } = useTranslation();
  if (target.source === "official") return null;
  return (
    <div className={styles.callout}>
      <ShieldAlert size={16} className={styles.calloutIcon} aria-hidden />
      <div>
        <p>
          {target.source === "thirdParty"
            ? t("plugins.install.notOfficial", { name: target.name, author: target.author })
            : t("plugins.install.notFromMarket", { name: target.name })}
        </p>
        <p className={styles.calloutRisk}>
          {t("plugins.install.leakRisk")}
          <br />
          {t("plugins.install.trustOnly")}
        </p>
      </div>
    </div>
  );
}

/** What the plugin may do, shown before the user agrees. High-risk permissions are marked. */
function InstallDetails({
  target,
  agreed,
  onAgree,
}: {
  target: InstallTarget;
  agreed: boolean;
  onAgree: (agreed: boolean) => void;
}) {
  const { t } = useTranslation();
  const official = target.source === "official";
  const nothingAsked =
    target.permissions.length === 0 && target.storageMb === null && !target.runsInBackground;
  return (
    <div className={styles.details}>
      {nothingAsked ? (
        <p>{t("plugins.install.noPermissions")}</p>
      ) : (
        <div className={styles.asks}>
          <p className={styles.listLabel}>{t("plugins.install.permissions")}</p>
          <ol className={styles.permissions}>
            {target.permissions.map((p) => (
              <li key={p}>
                <span>{t(`plugins.permissions.${permissionKey(p)}`, { defaultValue: p })}</span>
                {HIGH_RISK.has(p) ? (
                  <span className={styles.risk}>{t("plugins.install.highRisk")}</span>
                ) : null}
              </li>
            ))}
            {target.storageMb !== null ? (
              <li>{t("plugins.install.storage", { mb: target.storageMb })}</li>
            ) : null}
            {target.runsInBackground ? <li>{t("plugins.install.background")}</li> : null}
          </ol>
        </div>
      )}
      <label className={styles.consent}>
        <input type="checkbox" checked={agreed} onChange={(e) => onAgree(e.target.checked)} />
        <span>{official ? t("plugins.install.agree") : t("plugins.install.agreeRisk")}</span>
      </label>
    </div>
  );
}
