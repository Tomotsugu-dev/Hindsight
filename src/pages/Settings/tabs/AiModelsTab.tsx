import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Bot, Cloud, Download, Server } from "lucide-react";
import { PageToc, type PageTocItem } from "../../../components/PageToc/PageToc";
import { useAiSettings } from "../../AISettings/shared/useAiSettings";
import { RuntimeSection } from "./aiModels/RuntimeSection";
import { MyModelsSection } from "./aiModels/MyModelsSection";
import { CatalogSection } from "./aiModels/CatalogSection";
import { CloudApiSection, type CloudEditor } from "./aiModels/CloudApiSection";
import { useEngineStatus } from "./aiModels/useEngineStatus";
import { useLocalModels } from "./aiModels/useLocalModels";
import styles from "../../AISettings/AISettings.module.css";
import layout from "./AiModelsTab.module.css";

/** The ids the table of contents jumps to. */
const TOC_IDS = {
  runtime: "ai-models-runtime",
  mine: "ai-models-mine",
  catalog: "ai-models-catalog",
  cloud: "ai-models-cloud",
} as const;

/** Scroll a section to the top after this render, without motion if the user asked for less. */
function scrollToSection(id: string) {
  window.requestAnimationFrame(() => {
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    document
      .getElementById(id)
      ?.scrollIntoView({ behavior: reduce ? "auto" : "smooth", block: "start" });
  });
}

/**
 * Settings → AI models: every model the app can use, and what they need.
 *
 * From top to bottom: what local models need to run, the models that can be used now, the models
 * that can be downloaded, and the form for cloud API connections.
 */
export default function AiModelsTab() {
  const { t } = useTranslation();
  const { ai, updateAi } = useAiSettings();
  const engine = useEngineStatus();
  const models = useLocalModels();
  const [cloudEditor, setCloudEditor] = useState<CloudEditor>({ mode: "closed" });

  if (!ai) return null;

  const toc: PageTocItem[] = [
    { id: TOC_IDS.runtime, label: t("settings.aiModels.toc.runtime"), icon: Server },
    { id: TOC_IDS.mine, label: t("settings.aiModels.toc.mine"), icon: Bot },
    { id: TOC_IDS.catalog, label: t("settings.aiModels.toc.catalog"), icon: Download },
    { id: TOC_IDS.cloud, label: t("settings.aiModels.toc.cloud"), icon: Cloud },
  ];

  const openCloud = (next: CloudEditor) => {
    setCloudEditor(next);
    scrollToSection(TOC_IDS.cloud);
  };

  const deleteCloud = (id: string) => {
    updateAi({ cloud: ai.cloud.filter((c) => c.id !== id) });
    if (cloudEditor.mode === "edit" && cloudEditor.id === id) setCloudEditor({ mode: "closed" });
  };

  return (
    <div className={layout.layout}>
      <div className={styles.content}>
        <div id={TOC_IDS.runtime} className={layout.anchor}>
          <RuntimeSection engine={engine} />
        </div>
        <div id={TOC_IDS.mine} className={layout.anchor}>
          <MyModelsSection
            models={models}
            refreshEngine={engine.refresh}
            onEditCloud={(id) => openCloud({ mode: "edit", id })}
            onDeleteCloud={deleteCloud}
            onAddCloud={() => openCloud({ mode: "new" })}
            onGoToCatalog={() => scrollToSection(TOC_IDS.catalog)}
          />
        </div>
        <div id={TOC_IDS.catalog} className={layout.anchor}>
          <CatalogSection models={models} />
        </div>
        <div id={TOC_IDS.cloud} className={layout.anchor}>
          <CloudApiSection editor={cloudEditor} onEditorChange={setCloudEditor} />
        </div>
      </div>
      <PageToc title={t("settings.aiModels.toc.title")} items={toc} />
    </div>
  );
}
