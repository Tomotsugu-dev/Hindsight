import { useEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { AlertCircle, Check, Info, ListFilter, Lock, Plus, Trash2 } from "lucide-react";
import type { AiProfileModule, VisibilityProfile } from "../../api/hindsight";
import { useCategories } from "../../state/categories";
import {
  nextProfileName,
  profileProblems,
  sortProfiles,
  useAiProfiles,
} from "../../state/aiProfiles";
import { Button } from "../Button/Button";
import { IconButton } from "../Button/IconButton";
import { CategoryChipMultiSelect } from "../FormControls/CategoryChipMultiSelect";
import { Section } from "../FormLayout/Section";
import { SimplePicker } from "../SimplePicker/SimplePicker";
import { TextInput } from "../TextInput/TextInput";
import styles from "./AiProfilesCard.module.css";

const MODULES: AiProfileModule[] = ["daily", "weekly", "chat"];

/** 下拉框里「默认分类」的值；配置 id 都以 p_ 开头，不会撞 */
const MAIN = "";

/** 网址里的 ?module= 不认识或没有时，看日报的。 */
function moduleOf(value: string | null): AiProfileModule {
  return MODULES.find((m) => m === value) ?? "daily";
}

/**
 * AI 设置 → 常规 顶部的「AI 能看到的数据」：日报、周报、对话各有一组配置，在这里起名、选分类、
 * 新建、删除；日报和周报还能选自动生成用哪套。看哪个功能记在网址的 ?module= 里，
 * 别的页面的齿轮带着它跳过来。
 */
export function AiProfilesCard() {
  const { t } = useTranslation();
  const [params, setParams] = useSearchParams();
  const module = moduleOf(params.get("module"));
  // 从齿轮跳过来时描一圈边，一会儿就退掉，看得出该改哪里
  const [arrived, setArrived] = useState(() => params.has("module"));

  useEffect(() => {
    if (!arrived) return;
    const id = window.setTimeout(() => setArrived(false), 1600);
    return () => window.clearTimeout(id);
  }, [arrived]);

  const choose = (next: AiProfileModule) => setParams({ module: next }, { replace: true });

  const switcher = (
    <div className={styles.switcher} role="group" aria-label={t("aiProfiles.moduleGroup")}>
      {MODULES.map((m) => (
        <button
          key={m}
          type="button"
          className={`${styles.switchItem} ${m === module ? styles.switchActive : ""}`}
          aria-pressed={m === module}
          onClick={() => choose(m)}
        >
          {t(`aiProfiles.feature.${m}`)}
        </button>
      ))}
    </div>
  );

  return (
    <div className={`${styles.wrap} ${arrived ? styles.arrived : ""}`}>
      <Section
        title={t("aiProfiles.pickerTitle")}
        icon={ListFilter}
        description={t(`aiProfiles.intro.${module}`)}
        headerAction={switcher}
      >
        {/* 换功能时整块重来：没存下的改动属于上一个功能，不带过去 */}
        <ModuleEditor key={module} module={module} />
      </Section>
    </div>
  );
}

/**
 * 一个功能的配置。改动先放在这里，每改一下检查一遍：都没问题就存进设置，
 * 有问题的那几套标红，改好之前什么都不存。
 */
function ModuleEditor({ module }: { module: AiProfileModule }) {
  const { t } = useTranslation();
  const { categories } = useCategories();
  const { current, save } = useAiProfiles(module);
  const [profiles, setProfiles] = useState(current.profiles);
  const [autoProfile, setAutoProfile] = useState(current.autoProfile);
  // 进来时按名字排；新建的接在最后，改名时不跳位置
  const [order, setOrder] = useState(() => sortProfiles(current.profiles).map(([id]) => id));
  const [added, setAdded] = useState<string | null>(null);
  const newName = useRef<HTMLInputElement>(null);
  const problems = profileProblems(profiles);
  const blocked = Object.keys(problems).length > 0;
  // The default profile: every category except "Excluded from stats" and "Private"
  const defaultIds = categories.filter((c) => !c.builtin && c.id !== "private").map((c) => c.id);

  const commit = (nextProfiles: Record<string, VisibilityProfile>, nextAuto: string | null) => {
    setProfiles(nextProfiles);
    setAutoProfile(nextAuto);
    if (Object.keys(profileProblems(nextProfiles)).length > 0) return;
    const trimmed = Object.fromEntries(
      Object.entries(nextProfiles).map(([id, p]) => [id, { ...p, name: p.name.trim() }]),
    );
    save({ profiles: trimmed, autoProfile: module === "chat" ? null : nextAuto });
  };

  const add = () => {
    const id = `p_${crypto.randomUUID().replace(/-/g, "").slice(0, 8)}`;
    // 新的一套从默认分类起步
    const name = nextProfileName(profiles, (n) => t("aiProfiles.defaultName", { n }));
    commit({ ...profiles, [id]: { name, categories: defaultIds } }, autoProfile);
    setOrder((prev) => [...prev, id]);
    setAdded(id);
  };

  // 新建的那套在列表最下面，可能在视野外：整行滚进来，聚焦它的名字，直接就能改名
  useEffect(() => {
    const input = newName.current;
    if (!input) return;
    input.focus({ preventScroll: true });
    input.select();
    input.closest("[data-profile]")?.scrollIntoView({ block: "nearest" });
  }, [added]);

  const patch = (id: string, change: Partial<VisibilityProfile>) =>
    commit({ ...profiles, [id]: { ...profiles[id], ...change } }, autoProfile);

  const remove = (id: string) => {
    const next = { ...profiles };
    delete next[id];
    commit(next, autoProfile === id ? null : autoProfile);
    setOrder((prev) => prev.filter((x) => x !== id));
  };

  const hasAuto = module !== "chat";
  const autoOptions = [
    { value: MAIN, label: t("aiProfiles.main") },
    ...order.map((id) => ({
      value: id,
      label: profiles[id].name.trim() || t("aiProfiles.unnamed"),
    })),
  ];
  const autoBadge = <span className={styles.badge}>{t("aiProfiles.autoLabel")}</span>;

  return (
    <>
      {module === "weekly" ? (
        <p className={`${styles.line} ${styles.note}`}>
          <Info size={14} strokeWidth={2} aria-hidden />
          <span>{t("aiProfiles.weeklyNote")}</span>
        </p>
      ) : null}

      {hasAuto ? (
        <div className={`${styles.line} ${styles.autoRow}`}>
          <span className={styles.autoLabel}>{t("aiProfiles.autoLabel")}</span>
          <SimplePicker
            value={autoProfile ?? MAIN}
            options={autoOptions}
            onChange={(v) => commit(profiles, v === MAIN ? null : v)}
            size="md"
          />
        </div>
      ) : null}

      <div className={`${styles.line} ${styles.profile}`}>
        <div className={styles.nameCol}>
          <span className={styles.mainName}>
            <Lock size={13} strokeWidth={2} aria-hidden />
            {t("aiProfiles.main")}
          </span>
          {hasAuto && autoProfile == null ? autoBadge : null}
        </div>
        <div className={styles.chipsCol}>
          <CategoryChipMultiSelect selectedIds={defaultIds} />
        </div>
        {/* Holds the delete button's place, so the chips line up with the rows below */}
        <span className={styles.deleteSpace} aria-hidden />
      </div>

      {order.map((id) => {
        const p = profiles[id];
        const problem = problems[id];
        const nameWrong =
          problem === "aiProfiles.errors.noName" || problem === "aiProfiles.errors.sameName";
        return (
          <div key={id} className={`${styles.line} ${styles.profile}`} data-profile>
            <div className={styles.nameCol}>
              <TextInput
                ref={id === added ? newName : undefined}
                className={`${styles.name} ${nameWrong ? styles.invalid : ""}`}
                value={p.name}
                placeholder={t("aiProfiles.namePlaceholder")}
                aria-label={t("aiProfiles.namePlaceholder")}
                aria-invalid={nameWrong}
                maxLength={30}
                onChange={(e) => patch(id, { name: e.target.value })}
              />
              {hasAuto && autoProfile === id ? autoBadge : null}
            </div>
            <div className={styles.chipsCol}>
              <CategoryChipMultiSelect
                selectedIds={p.categories}
                onChange={(next) => patch(id, { categories: next })}
              />
              {problem ? (
                <p className={styles.problem}>
                  <AlertCircle size={13} strokeWidth={2} aria-hidden />
                  {t(problem)}
                </p>
              ) : null}
            </div>
            <IconButton
              tone="danger"
              icon={Trash2}
              label={t("aiProfiles.delete")}
              onClick={() => remove(id)}
            />
          </div>
        );
      })}

      {order.length === 0 ? (
        <p className={`${styles.line} ${styles.empty}`}>{t("aiProfiles.empty")}</p>
      ) : null}

      <div className={`${styles.line} ${styles.foot}`}>
        <Button icon={Plus} onClick={add}>
          {t("aiProfiles.add")}
        </Button>
        {blocked ? (
          <span className={`${styles.status} ${styles.statusBlocked}`} role="status">
            <AlertCircle size={13} strokeWidth={2} aria-hidden />
            {t("aiProfiles.notSaved")}
          </span>
        ) : (
          <span className={`${styles.status} ${styles.statusSaved}`} role="status">
            <Check size={13} strokeWidth={2.4} aria-hidden />
            {t("aiProfiles.saved")}
          </span>
        )}
      </div>
    </>
  );
}
