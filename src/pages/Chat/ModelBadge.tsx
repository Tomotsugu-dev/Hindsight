import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronDown, Cloud, HardDrive } from "lucide-react";
import {
  api,
  SUMMARY_CLOUD_SENTINEL,
  type ExternalProfile,
  type ModelEntry,
} from "../../api/hindsight";
import { useSettings } from "../../state/settings";
import { profileLabel } from "../../utils/profileLabel";
import { logError } from "../../lib/logger";
import { chatCloudReady, chatLocalModelName, chatUsesCloud } from "./chatRouting";
import styles from "./ChatPage.module.css";

/**
 * 当前 Chat 模型 badge + 下拉选择器:
 * - 云端 = 琥珀警告色(数据出设备),本地 = 灰;
 * - 点开可切换:任一已保存的云端配置 / 当前未保存的云端配置 / 任一本地 GGUF。
 *   本地切换写独立的 chat 槽位(setStepModel "chat"),不影响段总结的模型选择;
 *   选云端配置会把四元组应用为当前激活配置(与设置页「应用配置」一致,
 *   段总结走云端时也会跟着换模型)。
 */
export default function ModelBadge() {
  const { t } = useTranslation();
  const { settings, update, reload } = useSettings();
  const [open, setOpen] = useState(false);
  const [localModels, setLocalModels] = useState<ModelEntry[]>([]);
  // 下载过主模型、但全因不支持工具调用被过滤 → 空态要说清"不是没模型,
  // 是模型不合格",否则用户对着空列表和自己下过的模型对不上号
  const [hasToollessOnly, setHasToollessOnly] = useState(false);
  const wrapRef = useRef<HTMLSpanElement>(null);

  // 点外面/Esc 关闭菜单
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  if (!settings) return null;
  const ai = settings.ai;
  const cloud = chatUsesCloud(ai);
  const localName = chatLocalModelName(ai);

  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (next) {
      // 打开时拉本地模型清单。过滤:mmproj 是投影文件;对话模板未声明工具
      // 调用的模型(Chat 的硬前提)不列——为什么不列在对话页黄条里有说明
      api
        .listLocalModels()
        .then((all) => {
          const mains = all.filter((m) => !m.isMmproj);
          const usable = mains.filter((m) => m.supportsTools !== false);
          setLocalModels(usable);
          setHasToollessOnly(usable.length === 0 && mains.length > 0);
        })
        .catch((e) => logError("chat.listModels", e));
    }
  };

  const select = async (value: string) => {
    setOpen(false);
    try {
      // 空 = 回自动;sentinel = 云端;文件名 = 本地。chat 纯文本不带 mmproj
      await api.setStepModel("chat", value, null);
      await reload();
    } catch (e) {
      logError("chat.setModel", e);
    }
  };

  /** 当前激活的云端四元组是否就是这个已存配置(打勾用,同设置页 chips) */
  const matchesActive = (p: ExternalProfile) =>
    p.provider === ai.externalProvider &&
    p.endpoint === ai.endpoint.trim() &&
    p.apiKey === ai.apiKey.trim() &&
    p.model === ai.model.trim();

  const selectProfile = async (p: ExternalProfile) => {
    setOpen(false);
    if (cloud && matchesActive(p)) return;
    try {
      if (!cloud) {
        // 本地→云端:chat 槽位切到 sentinel 顺带停掉本地 server(与云端项一致)
        await api.setStepModel("chat", SUMMARY_CLOUD_SENTINEL, null);
      }
      // 四元组 + chatMain + 总开关一次乐观补丁走 debounce 持久化;
      // update 每次带全量 ai 对象,最后一次 flush 自然盖掉上面的中间写
      update({
        ai: {
          ...ai,
          chatMain: SUMMARY_CLOUD_SENTINEL,
          externalEnabled: true,
          externalProvider: p.provider || "custom",
          endpoint: p.endpoint,
          apiKey: p.apiKey,
          model: p.model,
        },
      });
    } catch (e) {
      logError("chat.applyProfile", e);
    }
  };

  return (
    <span ref={wrapRef} className={styles.badgeWrap}>
      <button
        type="button"
        className={`${styles.badge} ${cloud ? styles.badgeCloud : ""}`}
        title={cloud ? ai.endpoint : undefined}
        onClick={toggle}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={t("chat.model.pickAria")}
      >
        {cloud ? (
          <Cloud size={12} strokeWidth={2.2} />
        ) : (
          <HardDrive size={12} strokeWidth={2.2} />
        )}
        {cloud
          ? t("chat.badge.cloud", { model: ai.model })
          : localName
            ? t("chat.badge.local", { model: localName })
            : t("chat.badge.none")}
        <ChevronDown size={12} strokeWidth={2.2} />
      </button>

      {open && (
        <div className={styles.badgeMenu} role="menu">
          {ai.externalProfiles.map((p, i) => (
            <button
              key={`${p.endpoint}|${p.model}|${i}`}
              type="button"
              role="menuitem"
              className={styles.badgeMenuItem}
              title={p.endpoint}
              onClick={() => void selectProfile(p)}
            >
              <Cloud size={12} strokeWidth={2.2} className={styles.badgeMenuCloudIcon} />
              <span className={styles.badgeMenuLabel}>{profileLabel(p)}</span>
              {cloud && matchesActive(p) && <Check size={12} strokeWidth={2.4} />}
            </button>
          ))}
          {/* 当前激活的云端配置没存成 profile 时,单独给一项,别让它没入口 */}
          {chatCloudReady(ai) && !ai.externalProfiles.some(matchesActive) && (
            <button
              type="button"
              role="menuitem"
              className={styles.badgeMenuItem}
              title={ai.endpoint}
              onClick={() => void select(SUMMARY_CLOUD_SENTINEL)}
            >
              <Cloud size={12} strokeWidth={2.2} className={styles.badgeMenuCloudIcon} />
              <span className={styles.badgeMenuLabel}>
                {t("chat.badge.cloud", { model: ai.model })}
              </span>
              {cloud && <Check size={12} strokeWidth={2.4} />}
            </button>
          )}
          {localModels.map((m) => (
            <button
              key={m.filename}
              type="button"
              role="menuitem"
              className={styles.badgeMenuItem}
              onClick={() => void select(m.filename)}
            >
              <HardDrive size={12} strokeWidth={2.2} />
              <span className={styles.badgeMenuLabel}>{m.filename}</span>
              {!cloud && localName === m.filename && (
                <Check size={12} strokeWidth={2.4} />
              )}
            </button>
          ))}
          {localModels.length === 0 &&
            ai.externalProfiles.length === 0 &&
            !chatCloudReady(ai) && (
              <p className={styles.badgeMenuEmpty}>
                {hasToollessOnly
                  ? t("chat.model.noToolUseModels")
                  : t("chat.model.empty")}
              </p>
            )}
        </div>
      )}
    </span>
  );
}
