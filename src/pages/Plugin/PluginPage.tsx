import { useEffect, useRef } from "react";
import { Link, useParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { ROUTES } from "../../config/nav";
import { EmptyHint } from "../../components/EmptyHint/EmptyHint";
import { usePluginPages } from "../../state/pluginPages";
import { forwardCalls, PAGE_SANDBOX } from "../../plugin-page/forwarder";
import styles from "./PluginPage.module.css";

/** A plugin page inside the main window, at /plugins/<plugin id>. A thin Hindsight bar on top
 * shows whose page it is. See docs/design/plugins/en/runtime.md. */
export default function PluginPage() {
  const { id } = useParams();
  const { t } = useTranslation();
  const pages = usePluginPages();
  const page = pages?.find((p) => p.id === id);
  const frameRef = useRef<HTMLIFrameElement>(null);

  const pageId = page?.id;
  const url = page?.url;
  useEffect(() => {
    const frame = frameRef.current;
    if (!frame || !pageId || !url) return;
    return forwardCalls(frame, pageId, url);
  }, [pageId, url]);

  if (pages === null) return null;
  if (!page) return <EmptyHint message={t("plugins.page.gone")} />;

  return (
    <div className={styles.page}>
      <header className={styles.bar}>
        <span className={styles.title}>
          {page.name}
          <span className={styles.kind}> · {t("plugins.page.kind")}</span>
        </span>
        <Link className={styles.manage} to={ROUTES.settingsPlugins}>
          {t("plugins.page.manage")}
        </Link>
      </header>
      {/* key: a new version gets a new iframe, and the forwarder follows it */}
      <iframe
        key={page.url}
        ref={frameRef}
        className={styles.frame}
        sandbox={PAGE_SANDBOX}
        src={page.url}
        title={page.name}
      />
    </div>
  );
}
