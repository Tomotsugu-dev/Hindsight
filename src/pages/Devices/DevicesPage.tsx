import { useEffect, useRef, useState, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowLeftRight,
  Check,
  ChevronRight,
  Cloud,
  CloudOff,
  Copy,
  Eye,
  EyeOff,
  Loader2,
  LogIn,
  LogOut,
  Pencil,
  RefreshCw,
  Server,
  Settings2,
  Trash2,
  type LucideIcon,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { listen } from "@tauri-apps/api/event";
import { useDeviceFilter, type Device } from "../../state/deviceFilter";
import { useCaptureStatus } from "../../hooks/useCaptureStatus";
import { useSettings } from "../../state/settings";
import { AppearancePicker } from "../../components/AppearancePicker/AppearancePicker";
import { ForgetRemoteDeviceDialog } from "../../components/ForgetRemoteDeviceDialog/ForgetRemoteDeviceDialog";
import { resolveCategoryIcon } from "../../config/categoryIcons";
import { logError } from "../../lib/logger";
import { api, OAUTH_URL_EVENT, type AuthState, type SyncStatus } from "../../api/hindsight";
import { Toggle } from "../../components/FormControls/Toggle";
import { SyncOptInDialog } from "./SyncOptInDialog";
import {
  connectErrorKind,
  NUTSTORE_DAV_URL,
  serviceKind,
  webdavAccountLabel,
} from "./syncAccount";
import styles from "./DevicesPage.module.css";

/** 可选上云三挡的字段名(settings 布尔键)。 */
type OptDatasetField = "syncAiSummaries" | "syncChatHistory" | "syncScreenMemory";

const OPT_DATASETS: { field: OptDatasetField; key: "summaries" | "chat" | "memory" }[] = [
  { field: "syncAiSummaries", key: "summaries" },
  { field: "syncChatHistory", key: "chat" },
  { field: "syncScreenMemory", key: "memory" },
];

export default function DevicesPage() {
  const { t } = useTranslation();
  const { devices, renameSelf, recolorSelf, reiconSelf, reload } = useDeviceFilter();
  const self = devices.find((d) => d.current);
  const others = devices.filter((d) => !d.current);

  // 当前正在弹"从云端移除"确认框的设备；null = 关闭
  const [forgetTarget, setForgetTarget] = useState<Device | null>(null);
  // 正在跑后端 forget 的设备 id（按 id 锁，避免多张卡都被禁用）
  const [forgetBusyId, setForgetBusyId] = useState<string | null>(null);

  // 同步账号：云同步卡片要用，「从云端移除」按钮也要看后端（WebDAV 上还不能移除）
  const [auth, setAuth] = useState<AuthState | null>(null);
  useEffect(() => {
    api.authStatus().then(setAuth).catch(() => setAuth(null));
  }, []);
  const refreshAuth = () => {
    api.authStatus().then(setAuth).catch(() => setAuth(null));
  };

  const runForget = async () => {
    if (!forgetTarget) return;
    const target = forgetTarget;
    setForgetTarget(null);
    setForgetBusyId(target.id);
    try {
      const deleted = await api.forgetRemoteDevice(target.id);
      window.alert(
        t("devices.forgetDialog.doneToast", { name: target.name, count: deleted }),
      );
      void reload();
    } catch (e) {
      logError("devices.forgetRemote", e);
      window.alert(
        t("devices.forgetDialog.error", {
          message: e instanceof Error ? e.message : String(e),
        }),
      );
    } finally {
      setForgetBusyId(null);
    }
  };

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>{t("devices.title")}</h1>
        <p className={styles.meta}>{t("devices.meta")}</p>
      </header>

      <CloudSyncCard auth={auth} setAuth={setAuth} refreshAuth={refreshAuth} />

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>{t("devices.sectionSelf")}</h2>
        {self && (
          <SelfRow
            device={self}
            onRename={renameSelf}
            onRecolor={recolorSelf}
            onReicon={reiconSelf}
          />
        )}
      </section>

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>{t("devices.sectionOthers")}</h2>
        {others.length === 0 ? (
          <div className={styles.empty}>{t("devices.emptyOthers")}</div>
        ) : (
          others.map((d) => (
            <OtherRow
              key={d.id}
              device={d}
              busy={forgetBusyId === d.id}
              canForget={auth?.backend !== "webdav"}
              onForget={() => setForgetTarget(d)}
            />
          ))
        )}
      </section>

      <ForgetRemoteDeviceDialog
        open={forgetTarget !== null}
        deviceName={forgetTarget?.name ?? ""}
        onConfirm={runForget}
        onCancel={() => setForgetTarget(null)}
      />
    </div>
  );
}

// 把时间戳格式化成相对时间（"刚刚"/"X 分钟前"/...），用 i18n 的 relative 命名空间
function useFmtRelative() {
  const { t } = useTranslation();
  return (iso: string): string => {
    const ts = new Date(iso).getTime();
    if (Number.isNaN(ts)) return t("devices.relative.justNow");
    const diff = Date.now() - ts;
    if (diff < 60_000) return t("devices.relative.justNow");
    if (diff < 3_600_000)
      return t("devices.relative.minutesAgo", {
        count: Math.floor(diff / 60_000),
      });
    if (diff < 86_400_000)
      return t("devices.relative.hoursAgo", {
        count: Math.floor(diff / 3_600_000),
      });
    return t("devices.relative.daysAgo", {
      count: Math.floor(diff / 86_400_000),
    });
  };
}

function CloudSyncCard({
  auth,
  setAuth,
  refreshAuth,
}: {
  auth: AuthState | null;
  setAuth: (next: AuthState | null) => void;
  refreshAuth: () => void;
}) {
  const { t } = useTranslation();
  const fmtRelative = useFmtRelative();
  const { settings, update } = useSettings();
  const { reload: reloadDevices } = useDeviceFilter();
  const [sync, setSync] = useState<SyncStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [syncBusy, setSyncBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [setupOpen, setSetupOpen] = useState(false);
  // 「选择同步服务」面板：没登录时一直开着，登录后点「更换」才开。
  // 面板里显示两张服务卡，或者 WebDAV 的连接表单
  const [chooserOpen, setChooserOpen] = useState(false);
  const [chooserView, setChooserView] = useState<ChooserView>("cards");
  // 可选上云:待确认的数据集(开启前弹琥珀警告;关闭直接生效)
  const [optInPending, setOptInPending] = useState<OptDatasetField | null>(null);
  // OAuth 手动兜底:浏览器打开失败立即显示复制链接;"成功"但几秒内未完成也显示
  //(ShellExecute 成功不保证浏览器可见:关联损坏/提权/安全软件都静默失败)
  const [oauthUrl, setOauthUrl] = useState<string | null>(null);
  const [showOauthFallback, setShowOauthFallback] = useState(false);
  const [oauthCopied, setOauthCopied] = useState(false);

  useEffect(() => {
    const fetchSync = () => {
      api.syncStatus().then(setSync).catch(() => {});
    };
    fetchSync();
    const t = window.setInterval(fetchSync, 10_000);
    return () => window.clearInterval(t);
  }, []);

  // 没登录：打开选择面板；上次用的是 WebDAV 就直接显示 WebDAV 表单，地址和用户名已填好。
  // 登录后：收起所有面板。仅订阅这两个字段的变化，整个 auth 对象引用变更不应触发
  useEffect(() => {
    if (auth && !auth.signedIn) {
      setChooserOpen(true);
      setChooserView(auth.backend === "webdav" ? "webdav" : "cards");
    } else if (auth?.signedIn) {
      setChooserOpen(false);
      setSetupOpen(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [auth?.signedIn, auth?.backend]);

  const openChooser = (view: ChooserView) => {
    setChooserView(view);
    setChooserOpen(true);
  };
  const refreshSync = () => {
    api.syncStatus().then(setSync).catch(() => {});
  };

  const onSignIn = async () => {
    setBusy(true);
    setError(null);
    setOauthUrl(null);
    setShowOauthFallback(false);
    setOauthCopied(false);
    // 等授权 URL 事件:opened=false 立即亮兜底;opened=true 延时 6s 仍在等则亮
    let fallbackTimer: number | undefined;
    const unlisten = await listen<{ url: string; opened: boolean }>(
      OAUTH_URL_EVENT,
      (e) => {
        setOauthUrl(e.payload.url);
        if (!e.payload.opened) {
          setShowOauthFallback(true);
        } else {
          fallbackTimer = window.setTimeout(() => setShowOauthFallback(true), 6000);
        }
      },
    );
    try {
      const next = await api.signInWithGoogle();
      setAuth(next);
      refreshSync();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      refreshAuth();
    } finally {
      unlisten();
      if (fallbackTimer !== undefined) window.clearTimeout(fallbackTimer);
      setBusy(false);
      setOauthUrl(null);
      setShowOauthFallback(false);
    }
  };

  const onCopyOauthUrl = async () => {
    if (!oauthUrl) return;
    try {
      await navigator.clipboard.writeText(oauthUrl);
      setOauthCopied(true);
      window.setTimeout(() => setOauthCopied(false), 2000);
    } catch (e) {
      logError("devices.copyOauthUrl", e);
    }
  };

  const onSignOut = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.signOut();
      refreshAuth();
      refreshSync();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const onSyncNow = async () => {
    setSyncBusy(true);
    setError(null);
    try {
      await api.syncNow();
      refreshSync();
      // 拉到新的远端活动后，让 device 列表也刷一下
      void reloadDevices();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      refreshSync();
    } finally {
      setSyncBusy(false);
    }
  };

  // 派生值放到早返回之前，确保所有 hook 调用顺序在每次渲染都一致（rules-of-hooks）。
  // 「同步中」= 本地 promise 在途(点击瞬间即时反馈)|| 引擎标志(跳页重挂后
  // 由 10s 轮询 + 挂载即拉恢复——纯组件 state 会失忆,这就是之前"看似被打断"的根源)
  const syncing = syncBusy || (sync?.syncInFlight ?? false);
  const signedIn = auth?.signedIn ?? false;
  const configured = auth?.configured ?? false;
  // 用户改凭证后 auth.configured 可能没及时更新，所以 UI 也按本地 settings 算一遍
  const credsFilled = !!(
    settings?.googleClientId.trim() && settings?.googleClientSecret.trim()
  );
  const canSignIn = configured || credsFilled;
  const onWebdav = auth?.backend === "webdav";
  const serviceName = t(`devices.cloud.services.${serviceKind(auth)}.name`);
  const webdavAccount = webdavAccountLabel(
    auth?.webdavUrl ?? null,
    auth?.webdavUser ?? null,
  );
  // 选 Google：有 OAuth 凭证就直接登录，没有就先展开凭证面板
  const onPickGoogle = canSignIn ? onSignIn : () => setSetupOpen(true);
  // 凭证失效时的「重新登录」：Google 重走 OAuth，WebDAV 重新填密码
  const onReauth = onWebdav ? () => openChooser("webdav") : onSignIn;

  // 当填好凭证后，刷新一下 auth 让 configured 同步过来（用于按钮可点态）
  useEffect(() => {
    if (credsFilled && !configured && !signedIn) {
      api.authStatus().then(setAuth).catch(() => {});
    }
  }, [credsFilled, configured, signedIn, setAuth]);

  if (!settings) return null;

  // 后端 last_error 用稳定前缀分类：
  //   [CRED_EXPIRED] —— refresh_token 真失效 / AES 密文解不开 / scope 不足，必须用户重登
  //   [OUT_OF_SPACE] —— 云端空间满了，用户清理或扩容后自动继续
  //   [ACCOUNT_EXPIRED] —— 云端账号过期了（坚果云），用户续费后自动继续
  //   [TRANSIENT]    —— 网络抖动 / Drive 5xx / keyring 临时读失败，下一轮自动重试
  // 只有 CRED_EXPIRED 才把"退出"换成"重新登录"，避免一个网络抖动就催用户重登。
  const errorPrefixed = (prefix: string) =>
    signedIn && !!sync?.lastError && sync.lastError.startsWith(prefix);
  const authExpired = errorPrefixed("[CRED_EXPIRED]");
  const outOfSpace = errorPrefixed("[OUT_OF_SPACE]");
  const accountExpired = errorPrefixed("[ACCOUNT_EXPIRED]");
  const transientError = errorPrefixed("[TRANSIENT]");
  const lastErrorDisplay = sync?.lastError?.replace(
    /^\[(?:CRED_EXPIRED|OUT_OF_SPACE|ACCOUNT_EXPIRED|TRANSIENT)\]\s*/,
    "",
  );

  return (
    <div
      className={`${styles.syncCard} ${signedIn ? styles.syncCardConnected : ""}`}
    >
      <div className={styles.syncHeader}>
        <div
          className={
            signedIn
              ? styles.syncIcon
              : `${styles.syncIcon} ${styles.syncIconMuted}`
          }
        >
          {signedIn ? (
            <Cloud size={20} strokeWidth={1.6} />
          ) : (
            <CloudOff size={20} strokeWidth={1.6} />
          )}
        </div>
        <div className={styles.syncBody}>
          <div className={styles.syncTitle}>
            {signedIn
              ? t("devices.cloud.state.connectedTo", { name: serviceName })
              : t("devices.cloud.state.notSignedIn")}
          </div>
          <div className={styles.syncMeta}>
            {signedIn
              ? onWebdav
                ? webdavAccount
                : auth?.email ?? auth?.uid ?? ""
              : onWebdav
                ? t("devices.cloud.webdav.signedOutPrompt", {
                    account: webdavAccount,
                  })
                : t("devices.cloud.desc.choosePrompt")}
          </div>
          {signedIn && sync && (
            <div className={styles.syncStats}>
              {sync.pending > 0 ? (
                <>
                  <span
                    className={`${styles.syncDot} ${styles.syncDotPending}`}
                    aria-hidden
                  />
                  <span className={styles.syncStatPending}>
                    {t("devices.cloud.stats.pending", { count: sync.pending })}
                  </span>
                </>
              ) : sync.lastPushedAt ? (
                <>
                  <span className={styles.syncDot} aria-hidden />
                  <span className={styles.syncStatOk}>
                    {t("devices.cloud.stats.synced", {
                      when: fmtRelative(sync.lastPushedAt),
                    })}
                  </span>
                </>
              ) : (
                <span>{t("devices.cloud.stats.waitingFirst")}</span>
              )}
              {sync.deadLetter > 0 && (
                <span className={styles.syncStatErr}>
                  {t("devices.cloud.stats.deadLetter", {
                    count: sync.deadLetter,
                  })}
                </span>
              )}
            </div>
          )}
          {!signedIn && canSignIn && !setupOpen && !onWebdav && (
            <button
              type="button"
              className={styles.editCredsLink}
              onClick={() => setSetupOpen(true)}
            >
              {t("devices.cloud.editCreds")}
            </button>
          )}
        </div>
        <div className={styles.syncActions}>
          {auth == null ? (
            // auth 状态还没拿到——别渲染 connectBtn(蓝) 然后秒切到 smallBtn(灰)，
            // 改成同尺寸 skeleton 占位，等 authStatus resolve 完再 swap
            <div
              className={styles.syncActionsSkeleton}
              aria-hidden="true"
            />
          ) : signedIn ? (
            <>
              <button
                type="button"
                className={styles.smallBtn}
                onClick={() =>
                  chooserOpen ? setChooserOpen(false) : openChooser("cards")
                }
                disabled={busy}
                title={t("devices.cloud.actions.changeTitle")}
              >
                <ArrowLeftRight size={13} strokeWidth={1.85} />
                {t("devices.cloud.actions.change")}
              </button>
              <button
                type="button"
                className={styles.smallBtn}
                onClick={onSyncNow}
                disabled={syncing}
                title={t("devices.cloud.actions.syncNowTitle")}
              >
                <RefreshCw
                  size={13}
                  strokeWidth={1.85}
                  className={syncing ? styles.spinning : ""}
                />
                {syncing
                  ? t("devices.cloud.actions.syncing")
                  : t("devices.cloud.actions.syncNow")}
              </button>
              {/* 凭证失效时同时给两个按钮：
                  - 退出登录：清掉本机 auth_state，回到未登录状态（不动云端数据 / 本机数据）
                  - 重新登录：走 OAuth 重新拿 token
                  没有 authExpired 时只显示退出登录（不需要"重新登录"，因为已经登着）。 */}
              {authExpired && (
                <button
                  type="button"
                  className={`${styles.smallBtn} ${styles.smallBtnDanger}`}
                  onClick={onSignOut}
                  disabled={busy}
                  title={t("devices.cloud.actions.signOutTitle")}
                >
                  <LogOut size={13} strokeWidth={1.85} />
                  {t("devices.cloud.actions.signOut")}
                </button>
              )}
              <button
                type="button"
                className={`${styles.smallBtn} ${
                  authExpired ? styles.smallBtnAccent : styles.smallBtnDanger
                }`}
                onClick={authExpired ? onReauth : onSignOut}
                disabled={busy}
                title={
                  authExpired
                    ? t("devices.cloud.actions.reauthTitle")
                    : t("devices.cloud.actions.signOutTitle")
                }
              >
                {authExpired ? (
                  <>
                    <LogIn size={13} strokeWidth={1.85} />
                    {t("devices.cloud.actions.signIn")}
                  </>
                ) : (
                  <>
                    <LogOut size={13} strokeWidth={1.85} />
                    {t("devices.cloud.actions.signOut")}
                  </>
                )}
              </button>
            </>
          ) : null}
        </div>
      </div>

      <div
        className={`${styles.setupWrap} ${chooserOpen ? styles.setupWrapOpen : ""}`}
        aria-hidden={!chooserOpen}
      >
        <div className={styles.setupInner}>
          {chooserOpen && (
            <ServiceChooser
              current={signedIn ? (onWebdav ? "webdav" : "drive") : null}
              view={chooserView}
              onViewChange={setChooserView}
              googleLabel={
                busy
                  ? t("devices.cloud.actions.signingIn")
                  : canSignIn
                    ? t("devices.cloud.actions.signInWithGoogle")
                    : t("devices.cloud.actions.configureOAuth")
              }
              googleBusy={busy}
              onPickGoogle={onPickGoogle}
              initialUrl={auth?.webdavUrl ?? ""}
              initialUser={auth?.webdavUser ?? ""}
              onConnected={() => {
                setChooserOpen(false);
                setError(null);
                refreshAuth();
                refreshSync();
                void reloadDevices();
              }}
              onCollapse={signedIn ? () => setChooserOpen(false) : undefined}
            />
          )}
        </div>
      </div>

      <div
        className={`${styles.setupWrap} ${
          setupOpen && (!signedIn || onWebdav) ? styles.setupWrapOpen : ""
        }`}
        aria-hidden={!setupOpen || (signedIn && !onWebdav)}
      >
        <div className={styles.setupInner}>
          <SetupPanel
            clientId={settings.googleClientId}
            clientSecret={settings.googleClientSecret}
            onChangeId={(v) => update({ googleClientId: v })}
            onChangeSecret={(v) => update({ googleClientSecret: v })}
            collapsible={canSignIn}
            onCollapse={() => setSetupOpen(false)}
          />
        </div>
      </div>

      {/* ── OAuth 手动兜底:浏览器没弹出来时给用户一条活路 ── */}
      {busy && showOauthFallback && oauthUrl && (
        <div className={styles.oauthFallback}>
          <span className={styles.oauthFallbackText}>
            {t("devices.cloud.oauthFallback.hint")}
          </span>
          <button
            type="button"
            className={styles.smallBtn}
            onClick={() => void onCopyOauthUrl()}
          >
            {oauthCopied ? (
              <>
                <Check size={13} strokeWidth={2.2} />
                {t("devices.cloud.oauthFallback.copied")}
              </>
            ) : (
              <>
                <Copy size={13} strokeWidth={1.85} />
                {t("devices.cloud.oauthFallback.copy")}
              </>
            )}
          </button>
        </div>
      )}

      {/* ── 同步内容:可选上云三挡(仅登录后显示;开启前弹琥珀警告)── */}
      {signedIn && (
        <div className={styles.datasetBlock}>
          <div className={styles.datasetHeader}>
            {t("devices.cloud.datasets.title")}
          </div>
          <p className={styles.datasetHint}>{t("devices.cloud.datasets.hint")}</p>
          {OPT_DATASETS.map(({ field, key }) => (
            <div key={field} className={styles.datasetRow}>
              <div className={styles.datasetText}>
                <span className={styles.datasetLabel}>
                  {t(`devices.cloud.datasets.${key}.label`)}
                </span>
                <span className={styles.datasetDesc}>
                  {t(`devices.cloud.datasets.${key}.desc`)}
                </span>
              </div>
              <Toggle
                checked={settings[field]}
                onChange={(next) => {
                  if (next) {
                    setOptInPending(field);
                  } else {
                    update({ [field]: false });
                  }
                }}
                ariaLabel={t(`devices.cloud.datasets.${key}.label`)}
              />
            </div>
          ))}
        </div>
      )}
      {optInPending && (
        <SyncOptInDialog
          open
          datasetLabel={t(
            `devices.cloud.datasets.${
              OPT_DATASETS.find((d) => d.field === optInPending)?.key
            }.label`,
          )}
          body={t(
            `devices.cloud.datasets.${
              OPT_DATASETS.find((d) => d.field === optInPending)?.key
            }.warn`,
          )}
          onConfirm={() => {
            update({ [optInPending]: true });
            setOptInPending(null);
          }}
          onCancel={() => setOptInPending(null)}
        />
      )}

      {error && <div className={styles.syncError}>{error}</div>}
      {!error && signedIn && sync?.lastError && (
        <div className={styles.syncError}>
          {authExpired
            ? t("devices.errors.credExpired")
            : outOfSpace
              ? t("devices.errors.outOfSpace")
              : accountExpired
                ? t("devices.errors.accountExpired")
                : transientError
                  ? t("devices.errors.transient")
                  : lastErrorDisplay}
        </div>
      )}
    </div>
  );
}

function SetupPanel({
  clientId,
  clientSecret,
  onChangeId,
  onChangeSecret,
  collapsible,
  onCollapse,
}: {
  clientId: string;
  clientSecret: string;
  onChangeId: (v: string) => void;
  onChangeSecret: (v: string) => void;
  collapsible: boolean;
  onCollapse: () => void;
}) {
  const { t } = useTranslation();
  const [secretVisible, setSecretVisible] = useState(false);
  const open = (url: string) => {
    void openUrl(url).catch(() => {});
  };
  return (
    <div className={styles.setupPanel}>
      <div className={styles.setupHeader}>
        <Settings2 size={13} strokeWidth={2} />
        <span>{t("devices.setup.header")}</span>
        {collapsible && (
          <button
            type="button"
            className={styles.setupClose}
            onClick={onCollapse}
          >
            {t("devices.setup.collapse")}
          </button>
        )}
      </div>
      <ol className={styles.setupSteps}>
        <li>
          <span className={styles.stepNum}>1</span>
          <div className={styles.stepBody}>
            <div className={styles.stepTitle}>
              {t("devices.setup.step1.title")}
            </div>
            <div className={styles.stepDesc}>
              {t("devices.setup.step1.bodyPrefix")}
              <span className={styles.cueBtn}>
                {t("devices.setup.step1.cueEnable")}
              </span>
              {t("devices.setup.step1.bodySuffix")}
            </div>
            <button
              type="button"
              className={styles.stepBtn}
              onClick={() =>
                open(
                  "https://console.cloud.google.com/apis/library/drive.googleapis.com",
                )
              }
            >
              {t("devices.setup.step1.openLink")}{" "}
              <ChevronRight size={13} strokeWidth={2.25} />
            </button>
          </div>
        </li>
        <li>
          <span className={styles.stepNum}>2</span>
          <div className={styles.stepBody}>
            <div className={styles.stepTitle}>
              {t("devices.setup.step2.title")}
            </div>
            <div className={styles.stepDesc}>
              {t("devices.setup.step2.bodyPrefix")}
              <span className={styles.cueBtn}>
                {t("devices.setup.step2.cueStart")}
              </span>
              {t("devices.setup.step2.bodyMiddle")}
              <span className={styles.cueBtn}>
                {t("devices.setup.step2.cueCreate")}
              </span>
              {t("devices.setup.step2.bodySuffix")}
            </div>
            <button
              type="button"
              className={styles.stepBtn}
              onClick={() => open("https://console.cloud.google.com/auth/audience")}
            >
              {t("devices.setup.step2.openLink")}{" "}
              <ChevronRight size={13} strokeWidth={2.25} />
            </button>
          </div>
        </li>
        <li>
          <span className={styles.stepNum}>3</span>
          <div className={styles.stepBody}>
            <div className={styles.stepTitle}>
              {t("devices.setup.step3.title")}
            </div>
            <div className={styles.stepDesc}>
              {t("devices.setup.step3.bodyPrefix")}
              <span className={styles.cueLink}>
                {t("devices.setup.step3.cueCreateClient")}
              </span>
              {t("devices.setup.step3.bodySuffix")}
            </div>
            <button
              type="button"
              className={styles.stepBtn}
              onClick={() => open("https://console.cloud.google.com/auth/clients")}
            >
              {t("devices.setup.step3.openLink")}{" "}
              <ChevronRight size={13} strokeWidth={2.25} />
            </button>
          </div>
        </li>
        <li>
          <span className={styles.stepNum}>4</span>
          <div className={styles.stepBody}>
            <div className={styles.stepTitle}>
              {t("devices.setup.step4.title")}
            </div>
            <label className={styles.credField}>
              <span className={styles.credLabel}>
                {t("devices.setup.step4.clientIdLabel")}
              </span>
              <input
                type="text"
                className={styles.credInput}
                value={clientId}
                onChange={(e) => onChangeId(e.target.value)}
                placeholder="xxxxxx.apps.googleusercontent.com"
                spellCheck={false}
                autoComplete="off"
              />
            </label>
            <label className={styles.credField}>
              <span className={styles.credLabel}>
                {t("devices.setup.step4.secretLabel")}
              </span>
              <div className={styles.credInputWrap}>
                <input
                  type={secretVisible ? "text" : "password"}
                  className={`${styles.credInput} ${styles.credInputWithBtn}`}
                  value={clientSecret}
                  onChange={(e) => onChangeSecret(e.target.value)}
                  placeholder="GOCSPX-..."
                  spellCheck={false}
                  autoComplete="off"
                />
                <button
                  type="button"
                  className={styles.credEyeBtn}
                  onClick={() => setSecretVisible((v) => !v)}
                  aria-label={
                    secretVisible
                      ? t("devices.setup.step4.hideSecret")
                      : t("devices.setup.step4.showSecret")
                  }
                  title={
                    secretVisible
                      ? t("devices.setup.step4.hide")
                      : t("devices.setup.step4.show")
                  }
                  tabIndex={-1}
                >
                  {secretVisible ? (
                    <EyeOff size={14} strokeWidth={1.85} />
                  ) : (
                    <Eye size={14} strokeWidth={1.85} />
                  )}
                </button>
              </div>
            </label>
          </div>
        </li>
      </ol>
    </div>
  );
}

/** 「选择同步服务」面板里显示两张服务卡，还是 WebDAV 的连接表单。 */
type ChooserView = "cards" | "webdav";

/** 选择同步服务：Google Drive 和 WebDAV 两张卡；选了 WebDAV 换成它的连接表单。 */
function ServiceChooser({
  current,
  view,
  onViewChange,
  googleLabel,
  googleBusy,
  onPickGoogle,
  initialUrl,
  initialUser,
  onConnected,
  onCollapse,
}: {
  /** 正在用的服务；没登录时是 null */
  current: "drive" | "webdav" | null;
  view: ChooserView;
  onViewChange: (view: ChooserView) => void;
  /** Google 卡的按钮文字：登录中 / 用 Google 登录 / 配置 OAuth */
  googleLabel: string;
  googleBusy: boolean;
  onPickGoogle: () => void;
  initialUrl: string;
  initialUser: string;
  onConnected: () => void;
  /** 登录后才能收起；没登录时面板一直开着 */
  onCollapse?: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className={styles.setupPanel}>
      <div className={styles.setupHeader}>
        <ArrowLeftRight size={13} strokeWidth={2} />
        <span>{t("devices.cloud.chooser.header")}</span>
        {view === "webdav" ? (
          <button
            type="button"
            className={styles.setupClose}
            onClick={() => onViewChange("cards")}
          >
            {t("devices.cloud.chooser.back")}
          </button>
        ) : (
          onCollapse && (
            <button type="button" className={styles.setupClose} onClick={onCollapse}>
              {t("devices.setup.collapse")}
            </button>
          )
        )}
      </div>
      {view === "cards" ? (
        <>
          <div className={styles.serviceGrid}>
            <ServiceCard
              icon={Cloud}
              name={t("devices.cloud.services.googleDrive.name")}
              desc={t("devices.cloud.services.googleDrive.desc")}
              current={current === "drive"}
              actionLabel={
                current === "drive" ? t("devices.cloud.chooser.changeAccount") : googleLabel
              }
              onAction={onPickGoogle}
              disabled={googleBusy}
            />
            <ServiceCard
              icon={Server}
              name={t("devices.cloud.services.webdav.name")}
              desc={t("devices.cloud.services.webdav.desc")}
              current={current === "webdav"}
              actionLabel={
                current === "webdav"
                  ? t("devices.cloud.chooser.changeAccount")
                  : t("devices.cloud.chooser.pick")
              }
              onAction={() => onViewChange("webdav")}
            />
          </div>
          {current && (
            <div className={styles.stepDesc}>{t("devices.cloud.chooser.notice")}</div>
          )}
        </>
      ) : (
        <WebDavForm
          initialUrl={initialUrl}
          initialUser={initialUser}
          onConnected={onConnected}
        />
      )}
    </div>
  );
}

function ServiceCard({
  icon: Icon,
  name,
  desc,
  current,
  actionLabel,
  onAction,
  disabled,
}: {
  icon: LucideIcon;
  name: string;
  desc: string;
  current: boolean;
  actionLabel: string;
  onAction: () => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  return (
    <div className={`${styles.serviceCard} ${current ? styles.serviceCardCurrent : ""}`}>
      <div className={styles.serviceName}>
        <Icon size={15} strokeWidth={1.85} />
        {name}
      </div>
      <div className={styles.serviceDesc}>{desc}</div>
      <div className={styles.serviceFoot}>
        {current && (
          <span className={styles.serviceCurrent}>
            <Check size={12} strokeWidth={2.4} />
            {t("devices.cloud.chooser.current")}
          </span>
        )}
        <button
          type="button"
          className={current ? styles.smallBtn : styles.connectBtn}
          onClick={onAction}
          disabled={disabled}
        >
          {actionLabel}
        </button>
      </div>
    </div>
  );
}

/** 连接 WebDAV：地址、用户名、应用密码。后端先试登录，连不上什么都不保存。 */
function WebDavForm({
  initialUrl,
  initialUser,
  onConnected,
}: {
  initialUrl: string;
  initialUser: string;
  onConnected: () => void;
}) {
  const { t } = useTranslation();
  const [url, setUrl] = useState(initialUrl);
  const [user, setUser] = useState(initialUser);
  const [password, setPassword] = useState("");
  const [passwordVisible, setPasswordVisible] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const canConnect =
    url.trim() !== "" && user.trim() !== "" && password !== "" && !busy;

  const onConnect = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.connectWebdav(url.trim(), user.trim(), password);
      onConnected();
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      const kind = connectErrorKind(message);
      setError(
        kind === "other"
          ? t("devices.cloud.webdav.errors.other", { message })
          : t(`devices.cloud.webdav.errors.${kind}`),
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <form
      className={styles.davForm}
      onSubmit={(e) => {
        e.preventDefault();
        if (canConnect) void onConnect();
      }}
    >
      <div className={styles.stepDesc}>
        {t("devices.cloud.webdav.hint", { url: NUTSTORE_DAV_URL })}
      </div>
      <label className={styles.credField}>
        <span className={styles.credLabel}>{t("devices.cloud.webdav.url")}</span>
        <input
          type="url"
          className={styles.credInput}
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder={NUTSTORE_DAV_URL}
          spellCheck={false}
          autoComplete="off"
        />
      </label>
      <label className={styles.credField}>
        <span className={styles.credLabel}>{t("devices.cloud.webdav.user")}</span>
        <input
          type="text"
          className={styles.credInput}
          value={user}
          onChange={(e) => setUser(e.target.value)}
          spellCheck={false}
          autoComplete="off"
        />
      </label>
      <label className={styles.credField}>
        <span className={styles.credLabel}>{t("devices.cloud.webdav.password")}</span>
        <div className={styles.credInputWrap}>
          <input
            type={passwordVisible ? "text" : "password"}
            className={`${styles.credInput} ${styles.credInputWithBtn}`}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            spellCheck={false}
            autoComplete="off"
          />
          <button
            type="button"
            className={styles.credEyeBtn}
            onClick={() => setPasswordVisible((v) => !v)}
            aria-label={
              passwordVisible
                ? t("devices.cloud.webdav.hidePassword")
                : t("devices.cloud.webdav.showPassword")
            }
            title={
              passwordVisible
                ? t("devices.setup.step4.hide")
                : t("devices.setup.step4.show")
            }
            tabIndex={-1}
          >
            {passwordVisible ? (
              <EyeOff size={14} strokeWidth={1.85} />
            ) : (
              <Eye size={14} strokeWidth={1.85} />
            )}
          </button>
        </div>
      </label>
      {error && <div className={styles.syncError}>{error}</div>}
      <button
        type="submit"
        className={`${styles.connectBtn} ${styles.davSubmit}`}
        disabled={!canConnect}
      >
        {busy ? (
          <Loader2 size={13} strokeWidth={2} className={styles.spinning} />
        ) : (
          <LogIn size={13} strokeWidth={2} />
        )}
        {busy
          ? t("devices.cloud.webdav.connecting")
          : t("devices.cloud.webdav.connect")}
      </button>
    </form>
  );
}

function SelfRow({
  device,
  onRename,
  onRecolor,
  onReicon,
}: {
  device: Device;
  onRename: (name: string) => void;
  onRecolor: (color: string) => void;
  onReicon: (icon: string) => void;
}) {
  const { t } = useTranslation();
  const { status } = useCaptureStatus();
  const fmtRelative = useFmtRelative();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(device.name);
  const [pickerOpen, setPickerOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const Icon = resolveCategoryIcon(device.icon);

  useEffect(() => {
    if (editing) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [editing]);

  useEffect(() => {
    if (!editing) setDraft(device.name);
  }, [device.name, editing]);

  const commitName = () => {
    const trimmed = draft.trim();
    if (trimmed && trimmed !== device.name) {
      onRename(trimmed);
    } else {
      setDraft(device.name);
    }
    setEditing(false);
  };

  const cancelName = () => {
    setDraft(device.name);
    setEditing(false);
  };

  const styleVar = { "--cat-color": device.color } as CSSProperties;
  // 用真实时间戳走相对时间格式化——写死"刚刚"的话采集停了几小时也显示"刚刚"
  const lastSeen = status?.lastCaptureAt ? fmtRelative(status.lastCaptureAt) : "—";
  const todayCount = status?.todayCount ?? 0;

  return (
    <div className={styles.deviceRow} style={styleVar}>
      <div className={styles.deviceIconWrap}>
        <button
          type="button"
          className={styles.deviceIconBtn}
          onClick={() => setPickerOpen((v) => !v)}
          aria-label={t("devices.self.appearanceAria")}
          title={t("devices.self.appearanceTitle")}
        >
          <Icon size={28} strokeWidth={1.85} />
        </button>
        {pickerOpen && (
          <AppearancePicker
            color={device.color}
            icon={device.icon}
            onColorChange={onRecolor}
            onIconChange={(i) => {
              onReicon(i);
              setPickerOpen(false);
            }}
            onDismiss={() => setPickerOpen(false)}
          />
        )}
      </div>

      <div className={styles.deviceBody}>
        <div className={styles.deviceNameRow}>
          {editing ? (
            <input
              ref={inputRef}
              className={styles.deviceNameInput}
              value={draft}
              maxLength={32}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={commitName}
              onKeyDown={(e) => {
                if (e.key === "Enter") commitName();
                if (e.key === "Escape") cancelName();
              }}
            />
          ) : (
            <span
              className={styles.deviceName}
              role="button"
              tabIndex={0}
              onDoubleClick={() => setEditing(true)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  setEditing(true);
                }
              }}
              title={t("devices.self.doubleClickToRename")}
            >
              {device.name}
            </span>
          )}
          <span className={styles.tag}>{t("devices.self.tag")}</span>
        </div>
        <div className={styles.metaRow}>
          <span>
            {t("devices.self.lastActivity", { when: lastSeen })}
          </span>
          <span className={styles.dotSep}>·</span>
          <span>
            {t("devices.self.todayCount", { count: todayCount })}
          </span>
        </div>
      </div>

      <div className={styles.deviceActions}>
        <button
          type="button"
          className={styles.actionBtn}
          onClick={() => setEditing(true)}
          aria-label={t("devices.self.renameAria")}
          title={t("devices.self.renameTitle")}
        >
          <Pencil size={14} strokeWidth={1.85} />
        </button>
      </div>
    </div>
  );
}

function OtherRow({
  device,
  busy,
  canForget,
  onForget,
}: {
  device: Device;
  /** 正在跑 forget_remote_device —— 按钮 disable + 显示 spinner */
  busy: boolean;
  /** WebDAV 上还不能从云端移除设备，不显示按钮 */
  canForget: boolean;
  onForget: () => void;
}) {
  const { t } = useTranslation();
  const fmtRelative = useFmtRelative();
  const Icon = resolveCategoryIcon(device.icon);
  const styleVar = { "--cat-color": device.color } as CSSProperties;
  const when = device.lastSeenAt ? fmtRelative(device.lastSeenAt) : "—";
  return (
    <div className={styles.deviceRow} style={styleVar}>
      <div className={styles.deviceIconWrap}>
        <div className={styles.deviceIconBtn} aria-hidden>
          <Icon size={28} strokeWidth={1.85} />
        </div>
      </div>
      <div className={styles.deviceBody}>
        <div className={styles.deviceNameRow}>
          <span className={styles.deviceName}>{device.name}</span>
        </div>
        <div className={styles.metaRow}>
          <span>{t("devices.other.lastSync", { when })}</span>
        </div>
      </div>
      <div className={styles.deviceActions}>
        {canForget && (
          <button
            type="button"
            className={styles.actionBtn}
            onClick={onForget}
            disabled={busy}
            aria-label={t("devices.other.forgetAria", { name: device.name })}
            title={t("devices.other.forgetTitle")}
          >
            {busy ? (
              <Loader2 size={14} strokeWidth={1.85} className={styles.spinning} />
            ) : (
              <Trash2 size={14} strokeWidth={1.85} />
            )}
          </button>
        )}
      </div>
    </div>
  );
}
