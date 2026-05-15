import { useEffect, useRef, useState, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import {
  ChevronRight,
  Cloud,
  CloudOff,
  Eye,
  EyeOff,
  LogIn,
  LogOut,
  Pencil,
  RefreshCw,
  RotateCcw,
  Settings2,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useDeviceFilter, type Device } from "../../state/deviceFilter";
import { useCaptureStatus } from "../../hooks/useCaptureStatus";
import { useSettings } from "../../state/settings";
import { AppearancePicker } from "../../components/AppearancePicker/AppearancePicker";
import { resolveCategoryIcon } from "../../config/categoryIcons";
import { api, type AuthState, type SyncStatus } from "../../api/hindsight";
import styles from "./DevicesPage.module.css";

export default function DevicesPage() {
  const { t } = useTranslation();
  const { devices, renameSelf, recolorSelf, reiconSelf } = useDeviceFilter();
  const self = devices.find((d) => d.current);
  const others = devices.filter((d) => !d.current);

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>{t("devices.title")}</h1>
        <p className={styles.meta}>{t("devices.meta")}</p>
      </header>

      <CloudSyncCard />

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
          others.map((d) => <OtherRow key={d.id} device={d} />)
        )}
      </section>
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

function CloudSyncCard() {
  const { t } = useTranslation();
  const fmtRelative = useFmtRelative();
  const { settings, update } = useSettings();
  const { reload: reloadDevices } = useDeviceFilter();
  const [auth, setAuth] = useState<AuthState | null>(null);
  const [sync, setSync] = useState<SyncStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [syncBusy, setSyncBusy] = useState(false);
  const [forceBusy, setForceBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [setupOpen, setSetupOpen] = useState(false);

  useEffect(() => {
    api.authStatus().then(setAuth).catch(() => setAuth(null));
    const fetchSync = () => {
      api.syncStatus().then(setSync).catch(() => {});
    };
    fetchSync();
    const t = window.setInterval(fetchSync, 10_000);
    return () => window.clearInterval(t);
  }, []);

  // 没填凭证 → 默认展开设置面板；填好且未登录 → 收起
  // 仅订阅 configured / signedIn 两个字段的变化，整个 auth 对象引用变更不应触发
  useEffect(() => {
    if (auth && !auth.signedIn) {
      setSetupOpen(!auth.configured);
    } else if (auth?.signedIn) {
      setSetupOpen(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [auth?.configured, auth?.signedIn]);

  const refreshAuth = () => {
    api.authStatus().then(setAuth).catch(() => setAuth(null));
  };
  const refreshSync = () => {
    api.syncStatus().then(setSync).catch(() => {});
  };

  const onSignIn = async () => {
    setBusy(true);
    setError(null);
    try {
      const next = await api.signInWithGoogle();
      setAuth(next);
      refreshSync();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      refreshAuth();
    } finally {
      setBusy(false);
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
      reloadDevices();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      refreshSync();
    } finally {
      setSyncBusy(false);
    }
  };

  // 「强制完全重同步」：用户用来从「两端数据长期对不上」状态自愈。
  // 调用后端的 force_resync 命令，把 sync_cursor 重置、清掉所有失败 outbox 行、
  // 重新入队所有本地 local_date —— 重 push + 重 pull 一轮。
  // 命令本身幂等：连点 N 次和点 1 次效果相同。
  const onForceResync = async () => {
    // window.confirm 临时占位，避免引入新组件
    if (!window.confirm(t("devices.cloud.confirm.forceResync"))) return;
    setForceBusy(true);
    setError(null);
    try {
      const r = await api.forceResync();
      refreshSync();
      reloadDevices();
      if (r.syncError) {
        // SQL 状态清干净了但 push/pull 一轮有报错；展示报错给用户
        setError(r.syncError);
      } else {
        window.alert(
          t("devices.cloud.toast.forceResyncDone", {
            cleared: r.clearedDeadLetter,
            enqueued: r.enqueuedDays,
          }),
        );
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      refreshSync();
    } finally {
      setForceBusy(false);
    }
  };

  // 派生值放到早返回之前，确保所有 hook 调用顺序在每次渲染都一致（rules-of-hooks）。
  const signedIn = auth?.signedIn ?? false;
  const configured = auth?.configured ?? false;
  // 用户改凭证后 auth.configured 可能没及时更新，所以 UI 也按本地 settings 算一遍
  const credsFilled = !!(
    settings?.googleClientId.trim() && settings?.googleClientSecret.trim()
  );
  const canSignIn = configured || credsFilled;

  // 当填好凭证后，刷新一下 auth 让 configured 同步过来（用于按钮可点态）
  useEffect(() => {
    if (credsFilled && !configured && !signedIn) {
      api.authStatus().then(setAuth).catch(() => {});
    }
  }, [credsFilled, configured, signedIn]);

  if (!settings) return null;

  // 后端 last_error 用稳定前缀分类：
  //   [CRED_EXPIRED] —— refresh_token 真失效 / AES 密文解不开 / scope 不足，必须用户重登
  //   [TRANSIENT]    —— 网络抖动 / Drive 5xx / keyring 临时读失败，下个 30s tick 自动重试
  // 只有 CRED_EXPIRED 才把"退出"换成"重新登录"，避免一个网络抖动就催用户重登。
  const authExpired =
    signedIn &&
    !!sync?.lastError &&
    sync.lastError.startsWith("[CRED_EXPIRED]");
  const transientError =
    signedIn &&
    !!sync?.lastError &&
    sync.lastError.startsWith("[TRANSIENT]");
  const lastErrorDisplay = sync?.lastError?.replace(
    /^\[(?:CRED_EXPIRED|TRANSIENT)\]\s*/,
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
              ? t("devices.cloud.state.connected")
              : canSignIn
                ? t("devices.cloud.state.notSignedIn")
                : t("devices.cloud.state.notConfigured")}
          </div>
          <div className={styles.syncMeta}>
            {signedIn
              ? auth?.email ?? auth?.uid ?? ""
              : canSignIn
                ? t("devices.cloud.desc.signInPrompt")
                : t("devices.cloud.desc.configurePrompt")}
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
          {!signedIn && canSignIn && !setupOpen && (
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
                onClick={onSyncNow}
                disabled={syncBusy || forceBusy}
                title={t("devices.cloud.actions.syncNowTitle")}
              >
                <RefreshCw
                  size={13}
                  strokeWidth={1.85}
                  className={syncBusy ? styles.spinning : ""}
                />
                {syncBusy
                  ? t("devices.cloud.actions.syncing")
                  : t("devices.cloud.actions.syncNow")}
              </button>
              <button
                type="button"
                className={`${styles.smallBtn} ${styles.smallBtnDanger}`}
                onClick={onForceResync}
                disabled={syncBusy || forceBusy}
                title={t("devices.cloud.actions.forceResyncTitle")}
              >
                <RotateCcw
                  size={13}
                  strokeWidth={1.85}
                  className={forceBusy ? styles.spinning : ""}
                />
                {forceBusy
                  ? t("devices.cloud.actions.forceResyncing")
                  : t("devices.cloud.actions.forceResync")}
              </button>
              <button
                type="button"
                className={`${styles.smallBtn} ${styles.smallBtnSwap} ${
                  authExpired ? styles.smallBtnAccent : styles.smallBtnDanger
                }`}
                onClick={authExpired ? onSignIn : onSignOut}
                disabled={busy}
                title={
                  authExpired
                    ? t("devices.cloud.actions.reauthTitle")
                    : t("devices.cloud.actions.signOutTitle")
                }
              >
                <span
                  className={styles.smallBtnFace}
                  data-active={!authExpired}
                  aria-hidden={authExpired}
                >
                  <LogOut size={13} strokeWidth={1.85} />
                  {t("devices.cloud.actions.signOut")}
                </span>
                <span
                  className={styles.smallBtnFace}
                  data-active={authExpired}
                  aria-hidden={!authExpired}
                >
                  <LogIn size={13} strokeWidth={1.85} />
                  {t("devices.cloud.actions.signIn")}
                </span>
              </button>
            </>
          ) : canSignIn ? (
            <button
              type="button"
              className={styles.connectBtn}
              onClick={onSignIn}
              disabled={busy}
            >
              <LogIn size={13} strokeWidth={2} />
              {busy
                ? t("devices.cloud.actions.signingIn")
                : t("devices.cloud.actions.signInWithGoogle")}
            </button>
          ) : (
            <button
              type="button"
              className={styles.connectBtn}
              onClick={() => setSetupOpen(true)}
            >
              <Settings2 size={13} strokeWidth={2} />
              {t("devices.cloud.actions.configureOAuth")}
            </button>
          )}
        </div>
      </div>

      <div
        className={`${styles.setupWrap} ${
          setupOpen && !signedIn ? styles.setupWrapOpen : ""
        }`}
        aria-hidden={!setupOpen || signedIn}
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

      {error && <div className={styles.syncError}>{error}</div>}
      {!error && signedIn && sync?.lastError && (
        <div className={styles.syncError}>
          {authExpired
            ? t("devices.errors.credExpired")
            : transientError
              ? t("devices.errors.transient")
              : lastErrorDisplay}
        </div>
      )}
      {auth?.requiresRestart && (
        <div className={styles.syncError}>
          {t("devices.cloud.restartHint")}
          <button
            type="button"
            className={styles.smallBtn}
            style={{ marginLeft: 8 }}
            onClick={() => api.restartApp().catch(() => {})}
          >
            <RefreshCw size={13} strokeWidth={1.85} />
            {t("devices.cloud.actions.restartApp")}
          </button>
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
  const lastSeen = status?.lastCaptureAt ? t("devices.relative.justNow") : "—";
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

function OtherRow({ device }: { device: Device }) {
  const { t } = useTranslation();
  const Icon = resolveCategoryIcon(device.icon);
  const styleVar = { "--cat-color": device.color } as CSSProperties;
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
          <span>{t("devices.other.lastSync", { when: "—" })}</span>
        </div>
      </div>
    </div>
  );
}
