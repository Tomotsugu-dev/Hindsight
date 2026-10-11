import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";
import { Download } from "lucide-react";
import { api, type DownloadSources } from "../../../api/hindsight";
import { DEFAULT_DOWNLOAD_SOURCES } from "../../../config/downloadSources";
import { ROUTES } from "../../../config/nav";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { SimplePicker } from "../../../components/SimplePicker/SimplePicker";
import { useSettings } from "../../../state/settings";
import { logError } from "../../../lib/logger";
import styles from "./DownloadSourcesSection.module.css";

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

export function DownloadSourcesSection() {
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
      // Persist before the picker shows the new source, so downloads read the saved choice.
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
  return (
    <Section
      title={t("aiSettings.downloadSources.title")}
      description={t("aiSettings.downloadSources.description")}
      icon={Download}
    >
      <Row label={t("aiSettings.downloadSources.sourceLabel")}>
        <SimplePicker<DownloadSources["mode"]>
          value={sources.mode}
          disabled={saving}
          options={["official", "mirror", "custom"].map((mode) => ({
            value: mode as DownloadSources["mode"],
            label: t(`aiSettings.downloadSources.modes.${mode}`),
          }))}
          onChange={(mode) => void save({ mode })}
        />
      </Row>
      {sources.mode !== "custom" ? (
        <p className={styles.note}>{t(`aiSettings.downloadSources.${sources.mode}Description`)}</p>
      ) : (
        <>
          <Row
            label={t("aiSettings.downloadSources.huggingfaceLabel")}
            description={t("aiSettings.downloadSources.huggingfaceHelp")}
            block
          >
            <input
              type="url"
              className={styles.input}
              value={huggingfaceUrl}
              aria-label={t("aiSettings.downloadSources.huggingfaceLabel")}
              disabled={saving}
              spellCheck={false}
              onChange={(event) => setHuggingfaceUrl(event.target.value)}
            />
          </Row>
          <Row
            label={t("aiSettings.downloadSources.githubLabel")}
            description={t("aiSettings.downloadSources.githubHelp")}
            block
          >
            <input
              type="url"
              className={styles.input}
              value={githubProxyUrl}
              aria-label={t("aiSettings.downloadSources.githubLabel")}
              disabled={saving}
              spellCheck={false}
              onChange={(event) => setGithubProxyUrl(event.target.value)}
            />
          </Row>
          <Row
            label={t("aiSettings.downloadSources.nugetLabel")}
            description={t("aiSettings.downloadSources.nugetHelp")}
            block
          >
            <input
              type="url"
              className={styles.input}
              value={nugetUrl}
              aria-label={t("aiSettings.downloadSources.nugetLabel")}
              disabled={saving}
              spellCheck={false}
              onChange={(event) => setNugetUrl(event.target.value)}
            />
          </Row>
          <div className={styles.actions}>
            <button
              type="button"
              className={styles.saveButton}
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
            </button>
          </div>
        </>
      )}
      {saving && (
        <p className={styles.note} role="status">
          {t("aiSettings.downloadSources.saving")}
        </p>
      )}
      {error && (
        <p className={styles.error} role="alert">
          {error}
        </p>
      )}
    </Section>
  );
}

export function DownloadSourcesLink() {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const mode = settings?.downloadSources?.mode ?? "official";
  return (
    <p className={styles.sourceLink}>
      <span>
        {t("aiSettings.downloadSources.sourceLabel")}:{" "}
        {t(`aiSettings.downloadSources.modes.${mode}`)}
      </span>
      <Link to={ROUTES.aiSettings}>{t("aiSettings.downloadSources.change")}</Link>
    </p>
  );
}
