import { useCallback, useEffect, useState, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Check,
  FileUp,
  FolderOpen,
  Paintbrush,
  Palette,
  Trash2,
  TriangleAlert,
} from "lucide-react";
import { Section } from "../../../components/FormLayout/Section";
import { ConfirmDialog } from "../../../components/ConfirmDialog/ConfirmDialog";
import { EmptyHint } from "../../../components/EmptyHint/EmptyHint";
import { themesApi, type InstalledTheme, type Theme } from "../../../api/themes";
import {
  APP_THEMES,
  customChoice,
  customIdOf,
  getStoredTheme,
  setStoredTheme,
  type AppTheme,
  type ThemeChoice,
} from "../../../lib/theme";
import { chooseCustomTheme, loadThemes } from "../../../lib/themeLibrary";
import { logError } from "../../../lib/logger";
import { pickDescription } from "../../../utils/pickDescription";
import styles from "./AppearanceTab.module.css";

export default function AppearanceTab() {
  const { t } = useTranslation();
  // 主题是纯前端偏好（localStorage），不走后端 settings；本地 state 仅用于高亮选中项
  const [theme, setTheme] = useState<ThemeChoice>(getStoredTheme);
  /** 主题库里的主题；还没读完时是 null */
  const [library, setLibrary] = useState<InstalledTheme[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  /** 导入、选择、删除一次只做一件 */
  const [busy, setBusy] = useState(false);
  const [deleting, setDeleting] = useState<InstalledTheme | null>(null);

  const reload = useCallback(async () => {
    setLibrary(await loadThemes());
    setLoadError(null);
  }, []);

  // 打开外观页时读主题库；窗口重新获得焦点时再读，放进主题文件夹的主题马上出现
  useEffect(() => {
    const load = () => {
      reload().catch((e) => {
        logError("theme.load", e);
        setLoadError(String(e));
      });
    };
    load();
    window.addEventListener("focus", load);
    return () => window.removeEventListener("focus", load);
  }, [reload]);

  const act = async (action: () => Promise<void>) => {
    setBusy(true);
    setActionError(null);
    try {
      await action();
    } catch (e) {
      logError("theme.action", e);
      setActionError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const chooseBuiltIn = (next: AppTheme) => {
    setStoredTheme(next); // 立即写 <html> 的三个属性 + localStorage
    setTheme(next);
  };

  const chooseCustom = (next: Theme) =>
    act(async () => {
      await chooseCustomTheme(next);
      setTheme(customChoice(next.id));
    });

  const importTheme = () =>
    act(async () => {
      const path = await open({
        multiple: false,
        directory: false,
        filters: [{ name: t("settings.appearance.custom.fileFilter"), extensions: ["json"] }],
      });
      if (typeof path !== "string") return; // 用户取消了
      const imported = await themesApi.import(path);
      // 导入后直接用上，用户马上能看到效果
      await chooseCustomTheme(imported);
      await reload();
      setTheme(customChoice(imported.id));
    });

  const openFolder = () => {
    themesApi.openFolder().catch((e) => {
      logError("theme.openFolder", e);
      setActionError(String(e));
    });
  };

  const confirmDelete = () => {
    const target = deleting;
    setDeleting(null);
    if (!target) return;
    void act(async () => {
      await themesApi.delete(target.folderName);
      // 删的是正在用的主题：换回简约
      if (customIdOf(theme) === target.folderName) chooseBuiltIn("minimal");
      await reload();
    });
  };

  const activeId = customIdOf(theme);
  const activeEntry = library?.find((item) => item.folderName === activeId);
  // 选中的自定义主题在主题库里用不了，窗口现在显示的是简约
  const unavailable = activeId !== null && library !== null && !activeEntry?.theme;

  return (
    <>
      <Section
        title={t("settings.appearance.sectionTitle")}
        description={t("settings.appearance.description")}
        icon={Palette}
        bare
      >
        <div className={styles.grid}>
          {APP_THEMES.map((key) => {
            const active = key === theme;
            return (
              <button
                key={key}
                type="button"
                className={`${styles.card} ${active ? styles.cardActive : ""}`}
                onClick={() => chooseBuiltIn(key)}
                aria-pressed={active}
              >
                <span className={`${styles.preview} ${styles[`preview_${key}`]}`} aria-hidden>
                  <span className={styles.previewSidebar} />
                  <span className={styles.previewBody}>
                    <span className={styles.previewLine} />
                    {key === "flat" ? (
                      <>
                        <span className={styles.previewFlatChart}>
                          {Array.from({ length: 9 }, (_, index) => (
                            <span key={index} />
                          ))}
                        </span>
                        <span className={styles.previewFlatLists}>
                          <span />
                          <span />
                        </span>
                      </>
                    ) : (
                      <>
                        <span className={styles.previewLine} />
                        <span className={styles.previewCard} />
                      </>
                    )}
                  </span>
                </span>
                <span className={styles.meta}>
                  <span className={styles.name}>
                    {t(`settings.appearance.themes.${key}.name`)}
                    {active ? <Check size={14} strokeWidth={2.4} className={styles.check} /> : null}
                  </span>
                  <span className={styles.desc}>{t(`settings.appearance.themes.${key}.desc`)}</span>
                </span>
              </button>
            );
          })}
        </div>
      </Section>

      <Section
        title={t("settings.appearance.custom.title")}
        description={t("settings.appearance.custom.description")}
        icon={Paintbrush}
        bare
      >
        <div className={styles.body}>
          {/* 按钮不放标题行：设置页只有 656px 宽，放标题行会把说明挤成窄窄一列 */}
          <div className={styles.actions}>
            <button
              type="button"
              className={styles.btn}
              disabled={busy}
              onClick={() => void importTheme()}
            >
              <FileUp size={14} strokeWidth={1.85} />
              {t("settings.appearance.custom.import")}
            </button>
            <button type="button" className={styles.btn} onClick={openFolder}>
              <FolderOpen size={14} strokeWidth={1.85} />
              {t("settings.appearance.custom.openFolder")}
            </button>
          </div>
          {unavailable ? (
            <p className={styles.notice}>
              {activeEntry?.error
                ? t("settings.appearance.custom.invalid", {
                    id: activeId,
                    error: activeEntry.error,
                  })
                : t("settings.appearance.custom.missing", { id: activeId })}
            </p>
          ) : null}
          {actionError ? <p className={styles.error}>{actionError}</p> : null}
          {library === null ? (
            loadError ? (
              <p className={styles.error}>
                {t("settings.appearance.custom.loadFailed", { error: loadError })}
              </p>
            ) : (
              <p className={styles.hint}>{t("settings.appearance.custom.loading")}</p>
            )
          ) : library.length === 0 ? (
            <EmptyHint message={t("settings.appearance.custom.empty")} />
          ) : (
            <div className={styles.grid}>
              {library.map((item) =>
                item.theme ? (
                  <CustomThemeCard
                    key={item.folderName}
                    theme={item.theme}
                    active={item.folderName === activeId}
                    disabled={busy}
                    onChoose={(next) => void chooseCustom(next)}
                    onDelete={() => setDeleting(item)}
                  />
                ) : (
                  <BrokenThemeCard
                    key={item.folderName}
                    item={item}
                    disabled={busy}
                    onDelete={() => setDeleting(item)}
                  />
                ),
              )}
            </div>
          )}
        </div>
      </Section>

      <ConfirmDialog
        open={deleting !== null}
        variant="danger"
        title={
          deleting
            ? t("settings.appearance.custom.deleteTitle", {
                name: deleting.theme?.name ?? deleting.folderName,
              })
            : ""
        }
        message={
          deleting
            ? t("settings.appearance.custom.deleteMessage", { folder: deleting.folderName })
            : undefined
        }
        confirmLabel={t("settings.appearance.custom.delete")}
        onConfirm={confirmDelete}
        onCancel={() => setDeleting(null)}
      />
    </>
  );
}

/** 主题库里的一个主题：预览、名字、版本和作者、简介。没生效的变量列出来给作者看。 */
function CustomThemeCard({
  theme,
  active,
  disabled,
  onChoose,
  onDelete,
}: {
  theme: Theme;
  active: boolean;
  disabled: boolean;
  onChoose: (theme: Theme) => void;
  onDelete: () => void;
}) {
  const { t, i18n } = useTranslation();
  const description = pickDescription(theme.description, i18n.language);
  return (
    <div className={`${styles.card} ${active ? styles.cardActive : ""}`}>
      <button
        type="button"
        className={styles.choose}
        disabled={disabled}
        onClick={() => onChoose(theme)}
        aria-pressed={active}
      >
        <CustomPreview theme={theme} />
        <span className={styles.meta}>
          <span className={styles.name}>
            {theme.name}
            {active ? <Check size={14} strokeWidth={2.4} className={styles.check} /> : null}
          </span>
          <span className={styles.version}>
            v{theme.version}
            {theme.author ? ` · ${theme.author}` : ""}
          </span>
          {description ? <span className={styles.desc}>{description}</span> : null}
        </span>
      </button>
      {theme.ignoredTokens.length > 0 ? (
        <p className={styles.ignored}>
          {t("settings.appearance.custom.ignored", { names: theme.ignoredTokens.join(", ") })}
        </p>
      ) : null}
      <div className={styles.cardFoot}>
        <button type="button" className={styles.btnDanger} disabled={disabled} onClick={onDelete}>
          <Trash2 size={13} strokeWidth={1.85} />
          {t("settings.appearance.custom.delete")}
        </button>
      </div>
    </div>
  );
}

/** 读不出来的主题：文件夹名和原因。只能删。 */
function BrokenThemeCard({
  item,
  disabled,
  onDelete,
}: {
  item: InstalledTheme;
  disabled: boolean;
  onDelete: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className={`${styles.card} ${styles.cardBroken}`}>
      <span className={styles.meta}>
        <span className={styles.name}>
          <TriangleAlert size={14} strokeWidth={2} className={styles.brokenIcon} />
          {item.folderName}
        </span>
        <span className={styles.desc}>{item.error}</span>
      </span>
      <div className={styles.cardFoot}>
        <button type="button" className={styles.btnDanger} disabled={disabled} onClick={onDelete}>
          <Trash2 size={13} strokeWidth={1.85} />
          {t("settings.appearance.custom.delete")}
        </button>
      </div>
    </div>
  );
}

/** 自定义主题的小预览：主题设了的颜色用主题的，没设的用默认主题的配色（暗色主题用暗色配色）。 */
function CustomPreview({ theme }: { theme: Theme }) {
  const v = theme.tokens;
  const colors = {
    "--preview-sidebar": v["--sidebar-gradient"],
    "--preview-body": v["--surface-app"],
    "--preview-card": v["--surface-card"],
    "--preview-line": v["--border-strong"],
    "--preview-accent": v["--color-accent"],
  } as CSSProperties;
  const classes = [
    styles.preview,
    styles.preview_custom,
    theme.mode === "dark" ? styles.preview_customDark : "",
    theme.layout === "flat" ? styles.preview_customFlat : "",
  ];
  return (
    <span className={classes.join(" ")} style={colors} aria-hidden>
      <span className={styles.previewSidebar} />
      <span className={styles.previewBody}>
        <span className={styles.previewLine} />
        <span className={styles.previewLine} />
        <span className={styles.previewCard} />
      </span>
    </span>
  );
}
