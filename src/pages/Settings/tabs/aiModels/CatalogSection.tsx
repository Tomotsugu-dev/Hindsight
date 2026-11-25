import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowUpDown, Download, FolderInput, ListFilter, Package, Pause, Play } from "lucide-react";
import { Section } from "../../../../components/FormLayout/Section";
import { Button } from "../../../../components/Button/Button";
import {
  SimplePicker,
  type SimplePickerOption,
} from "../../../../components/SimplePicker/SimplePicker";
import { TextInput } from "../../../../components/TextInput/TextInput";
import type { ModelDownloadProgress, RecommendedModel } from "../../../../api/hindsight";
import {
  catalogList,
  visibleCatalog,
  COLLAPSED_COUNT,
  gb,
  type CatalogFilter,
  type CatalogSort,
} from "./catalog";
import { mmprojSaveAs } from "./localFiles";
import type { LocalModels } from "./useLocalModels";
import { CapTags, ModelLogo } from "./modelBits";
import legacy from "../../../AISettings/AISettings.module.css";
import s from "./aiModels.module.css";

const FILTERS: CatalogFilter[] = ["all", "vision", "reasoning", "small"];

/**
 * The recommended models that are not downloaded yet, with chips, a brand picker and an order.
 *
 * It shows the first few until the user asks for the rest. At the bottom are the two ways to add a
 * model that is not in the list: a Hugging Face repo, or a .gguf file on disk.
 */
export function CatalogSection({ models }: { models: LocalModels }) {
  const { t } = useTranslation();
  const [filter, setFilter] = useState<CatalogFilter>("all");
  const [brand, setBrand] = useState("all");
  const [sort, setSort] = useState<CatalogSort>("default");
  const [expanded, setExpanded] = useState(false);
  const [showCustom, setShowCustom] = useState(false);

  const { recommended, isInstalled, busyFiles, partials } = models;
  const busy = (rec: RecommendedModel) =>
    busyFiles.has(rec.mainFile) || (!!rec.mmprojFile && busyFiles.has(mmprojSaveAs(rec)));
  const paused = (rec: RecommendedModel) => !busy(rec) && rec.mainFile in partials;

  const notInstalled = recommended.filter((r) => !isInstalled(r));
  const list = catalogList(recommended, isInstalled, filter, brand, sort);
  const shown = visibleCatalog(list, expanded, (r) => busy(r) || paused(r));
  const hidden = list.length - shown.length;

  const brandOptions = useMemo<SimplePickerOption<string>[]>(() => {
    const brands = Array.from(new Set(recommended.map((r) => r.brand).filter(Boolean))).sort(
      (a, b) => a.localeCompare(b),
    );
    return [
      { value: "all", label: t("settings.aiModels.catalog.brandAll") },
      ...brands.map((b) => ({ value: b, label: b })),
    ];
  }, [recommended, t]);

  const sortOptions: SimplePickerOption<CatalogSort>[] = [
    { value: "default", label: t("settings.aiModels.catalog.sortDefault") },
    { value: "size-asc", label: t("aiSettings.models.toolbar.sortSizeAsc") },
    { value: "size-desc", label: t("aiSettings.models.toolbar.sortSizeDesc") },
    { value: "name", label: t("aiSettings.models.toolbar.sortName") },
    { value: "released", label: t("aiSettings.models.toolbar.sortReleased") },
  ];

  const chipLabel = (f: CatalogFilter) =>
    f === "all"
      ? t("settings.aiModels.catalog.all", { count: notInstalled.length })
      : t(`settings.aiModels.catalog.${f}`);

  return (
    <Section
      title={t("settings.aiModels.catalog.title")}
      description={t("settings.aiModels.catalog.desc")}
    >
      <div className={s.toolbar}>
        {FILTERS.map((f) => (
          <button
            key={f}
            type="button"
            className={s.chip}
            aria-pressed={filter === f}
            onClick={() => setFilter(f)}
          >
            {chipLabel(f)}
          </button>
        ))}
        <div className={s.toolbarEnd}>
          <SimplePicker<string>
            value={brand}
            options={brandOptions}
            onChange={setBrand}
            icon={ListFilter}
          />
          <SimplePicker<CatalogSort>
            value={sort}
            options={sortOptions}
            onChange={setSort}
            icon={ArrowUpDown}
          />
        </div>
      </div>
      {models.error?.place === "catalog" ? (
        <div className={`${legacy.engineError} ${s.error}`}>{models.error.message}</div>
      ) : null}

      <div className={s.list}>
        {shown.map((rec) => (
          <CatalogRow
            key={rec.mainFile}
            rec={rec}
            models={models}
            busy={busy(rec)}
            paused={paused(rec)}
          />
        ))}
      </div>
      {list.length === 0 ? (
        <div className={s.empty}>
          {notInstalled.length === 0 && recommended.length > 0
            ? t("settings.aiModels.catalog.allDone")
            : recommended.length > 0
              ? t("settings.aiModels.catalog.noMatch")
              : t("aiSettings.engine.loading")}
        </div>
      ) : null}

      <div className={s.foot}>
        {hidden > 0 || (expanded && list.length > COLLAPSED_COUNT) ? (
          <button
            type="button"
            className={s.linkBtn}
            aria-expanded={expanded}
            onClick={() => setExpanded(!expanded)}
          >
            {expanded
              ? t("settings.aiModels.catalog.showLess")
              : t("settings.aiModels.catalog.showMore", { count: hidden })}
          </button>
        ) : null}
        <span className={s.spacer} />
        <span>{t("settings.aiModels.catalog.notFound")}</span>
        <Button
          variant="outline"
          icon={Package}
          aria-expanded={showCustom}
          onClick={() => setShowCustom(!showCustom)}
        >
          {t("aiSettings.models.custom.expand")}
        </Button>
        <ImportButton models={models} />
      </div>
      {showCustom ? <CustomDownload models={models} /> : null}
    </Section>
  );
}

function CatalogRow({
  rec,
  models,
  busy,
  paused,
}: {
  rec: RecommendedModel;
  models: LocalModels;
  busy: boolean;
  paused: boolean;
}) {
  const { t } = useTranslation();
  const mmproj = rec.mmprojFile ? mmprojSaveAs(rec) : "";
  const activeFile = models.busyFiles.has(rec.mainFile) ? rec.mainFile : busy ? mmproj : "";
  const progress = activeFile ? models.progress[activeFile] : undefined;

  return (
    <div className={s.item}>
      <div className={s.row}>
        <ModelLogo rec={rec} />
        <div className={s.main}>
          <span className={s.name} title={rec.repo}>
            {rec.displayName}
            <CapTags caps={rec.caps} />
          </span>
          {busy && progress ? (
            <DownloadProgress progress={progress} isMmproj={activeFile === mmproj && !!mmproj} />
          ) : paused ? (
            <span className={s.meta}>
              {t("settings.aiModels.catalog.paused", {
                size: gb(models.partials[rec.mainFile] ?? 0),
              })}
            </span>
          ) : null}
        </div>
        <div className={s.side}>
          <span className={s.size}>{gb(rec.mainBytes + rec.mmprojBytes)} GB</span>
          {busy ? (
            <Button
              variant="outline"
              icon={Pause}
              onClick={() => activeFile && models.pause(activeFile)}
              title={t("aiSettings.models.card.pauseTooltip")}
            >
              {t("aiSettings.models.card.pause")}
            </Button>
          ) : (
            <Button
              variant="outline"
              tone="accent"
              icon={paused ? Play : Download}
              onClick={() => void models.download(rec)}
            >
              {paused ? t("aiSettings.models.card.resume") : t("aiSettings.models.card.download")}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}

/** A bar and "Main weights · 120.0 / 3200.0 MB". Without a total the bar slides instead of filling. */
function DownloadProgress({
  progress,
  isMmproj,
}: {
  progress: ModelDownloadProgress;
  isMmproj: boolean;
}) {
  const { t } = useTranslation();
  const mb = (bytes: number) => (bytes / 1024 / 1024).toFixed(1);
  const pct = progress.total ? Math.min(100, (progress.downloaded / progress.total) * 100) : null;
  return (
    <div className={s.progress}>
      <div className={s.track}>
        <div
          className={`${s.fill} ${pct == null ? s.fillUnknown : ""}`}
          style={pct == null ? undefined : { width: `${pct}%` }}
        />
      </div>
      <span className={s.meta}>
        {t(
          isMmproj
            ? "aiSettings.models.card.progressMmproj"
            : "aiSettings.models.card.progressMain",
        )}{" "}
        · {mb(progress.downloaded)} /{" "}
        {progress.total ? mb(progress.total) : t("aiSettings.models.card.progressUnknownTotal")}{" "}
        {t("aiSettings.models.card.progressUnit")}
      </span>
    </div>
  );
}

/** Copies .gguf files from disk into the models folder. While it copies, the button shows which file. */
function ImportButton({ models }: { models: LocalModels }) {
  const { t } = useTranslation();
  const current = models.importing.size > 0 ? Array.from(models.importing)[0] : "";
  const p = current ? models.progress[current] : undefined;
  const pct = p && p.total ? Math.floor((p.downloaded / p.total) * 100) : null;
  return (
    <Button
      variant="outline"
      icon={FolderInput}
      loading={!!current}
      onClick={() => void models.importFiles()}
      title={t("aiSettings.models.import.tooltip")}
    >
      {!current
        ? t("aiSettings.models.import.button")
        : pct != null
          ? t("aiSettings.models.import.importingPct", { name: current, pct })
          : t("aiSettings.models.import.importing", { name: current })}
    </Button>
  );
}

/** Downloads any GGUF from a Hugging Face repo. The file must load in llama.cpp; mmproj is for vision models. */
function CustomDownload({ models }: { models: LocalModels }) {
  const { t } = useTranslation();
  const [repo, setRepo] = useState("");
  const [mainFile, setMainFile] = useState("");
  const [mmprojFile, setMmprojFile] = useState("");
  const mainBusy = !!mainFile.trim() && models.busyFiles.has(mainFile.trim());
  const mmprojBusy = !!mmprojFile.trim() && models.busyFiles.has(mmprojFile.trim());
  const busy = mainBusy || mmprojBusy;
  const activeFile = mainBusy ? mainFile.trim() : mmprojBusy ? mmprojFile.trim() : "";
  const progress = activeFile ? models.progress[activeFile] : undefined;

  const fields = [
    { key: "repo", value: repo, set: setRepo },
    { key: "mainFile", value: mainFile, set: setMainFile },
    { key: "mmprojFile", value: mmprojFile, set: setMmprojFile },
  ];

  return (
    <div className={s.panel}>
      <span className={s.help}>{t("aiSettings.models.custom.hint")}</span>
      {fields.map((f) => (
        <label key={f.key} className={s.field}>
          <span className={s.label}>{t(`aiSettings.models.custom.${f.key}Label`)}</span>
          <TextInput
            value={f.value}
            placeholder={t(`aiSettings.models.custom.${f.key}Placeholder`)}
            onChange={(e) => f.set(e.target.value)}
            spellCheck={false}
            autoCorrect="off"
            autoCapitalize="off"
            disabled={busy}
          />
        </label>
      ))}
      {busy && progress ? <DownloadProgress progress={progress} isMmproj={mmprojBusy} /> : null}
      <div className={s.actions}>
        <Button
          variant="solid"
          tone="accent"
          icon={Download}
          loading={busy}
          disabled={busy || !repo.trim() || !mainFile.trim()}
          onClick={() => void models.downloadCustom(repo, mainFile, mmprojFile)}
        >
          {busy ? t("aiSettings.models.card.downloading") : t("aiSettings.models.card.download")}
        </Button>
      </div>
    </div>
  );
}
