import { useEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { AlertCircle, Check, Info, ListFilter, Lock, Plus, Trash2 } from "lucide-react";
import type { AutoProfiles, AutoReport, VisibilityProfile } from "../../api/hindsight";
import { useCategories } from "../../state/categories";
import {
  clearAuto,
  nextProfileName,
  profileProblems,
  sortProfiles,
  useAiVisibility,
} from "../../state/aiProfiles";
import { Button } from "../Button/Button";
import { IconButton } from "../Button/IconButton";
import { CategoryChipMultiSelect } from "../FormControls/CategoryChipMultiSelect";
import { Section } from "../FormLayout/Section";
import { SimplePicker } from "../SimplePicker/SimplePicker";
import { TextInput } from "../TextInput/TextInput";
import styles from "./AiProfilesCard.module.css";

const REPORTS: AutoReport[] = ["daily", "weekly"];

/** 下拉框里「默认分类」的值；配置 id 都以 p_ 开头，不会撞 */
const MAIN = "";

/**
 * "Data the AI can see" at the top of AI settings → General. There is one set of profiles for
 * every feature. Here the user names them, picks their categories, adds and deletes them, and
 * picks the one that daily and weekly reports each use when they run automatically.
 */
export function AiProfilesCard() {
  const { t } = useTranslation();
  const [params] = useSearchParams();
  // Outlined for a moment when a link from another page opens it, so the user sees where to look
  const [arrived, setArrived] = useState(() => params.get("focus") === "profiles");

  useEffect(() => {
    if (!arrived) return;
    const id = window.setTimeout(() => setArrived(false), 1600);
    return () => window.clearTimeout(id);
  }, [arrived]);

  return (
    <div className={`${styles.wrap} ${arrived ? styles.arrived : ""}`}>
      <Section
        title={t("aiProfiles.pickerTitle")}
        icon={ListFilter}
        description={t("aiProfiles.intro")}
      >
        <ProfilesEditor />
      </Section>
    </div>
  );
}

/**
 * 改动先放在这里，每改一下检查一遍：都没问题就存进设置，
 * 有问题的那几套标红，改好之前什么都不存。
 */
function ProfilesEditor() {
  const { t } = useTranslation();
  const { categories } = useCategories();
  const { current, save } = useAiVisibility();
  const [profiles, setProfiles] = useState(current.profiles);
  const [auto, setAuto] = useState(current.auto);
  // 进来时按名字排；新建的接在最后，改名时不跳位置
  const [order, setOrder] = useState(() => sortProfiles(current.profiles).map(([id]) => id));
  const [added, setAdded] = useState<string | null>(null);
  const newName = useRef<HTMLInputElement>(null);
  const problems = profileProblems(profiles);
  const blocked = Object.keys(problems).length > 0;
  // The default profile: every category except "Excluded from stats" and "Private"
  const defaultIds = categories.filter((c) => !c.builtin && c.id !== "private").map((c) => c.id);

  const commit = (nextProfiles: Record<string, VisibilityProfile>, nextAuto: AutoProfiles) => {
    setProfiles(nextProfiles);
    setAuto(nextAuto);
    if (Object.keys(profileProblems(nextProfiles)).length > 0) return;
    const trimmed = Object.fromEntries(
      Object.entries(nextProfiles).map(([id, p]) => [id, { ...p, name: p.name.trim() }]),
    );
    save({ profiles: trimmed, auto: nextAuto });
  };

  const add = () => {
    const id = `p_${crypto.randomUUID().replace(/-/g, "").slice(0, 8)}`;
    // 新的一套从默认分类起步
    const name = nextProfileName(profiles, (n) => t("aiProfiles.defaultName", { n }));
    commit({ ...profiles, [id]: { name, categories: defaultIds } }, auto);
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
    commit({ ...profiles, [id]: { ...profiles[id], ...change } }, auto);

  const remove = (id: string) => {
    const next = { ...profiles };
    delete next[id];
    commit(next, clearAuto(auto, id));
    setOrder((prev) => prev.filter((x) => x !== id));
  };

  // The profile a report really runs with: one that is gone means the default categories
  const autoOf = (report: AutoReport) => {
    const id = auto[report];
    return id != null && Object.prototype.hasOwnProperty.call(profiles, id) ? id : null;
  };

  const autoOptions = [
    { value: MAIN, label: t("aiProfiles.main") },
    ...order.map((id) => ({
      value: id,
      label: profiles[id].name.trim() || t("aiProfiles.unnamed"),
    })),
  ];

  /** Tags under a profile's name for the reports that use it when they run automatically. */
  const autoBadges = (id: string | null) => {
    const reports = REPORTS.filter((r) => autoOf(r) === id);
    if (reports.length === 0) return null;
    return (
      <div className={styles.badges}>
        {reports.map((r) => (
          <span key={r} className={styles.badge}>
            {t(`aiProfiles.autoBadge.${r}`)}
          </span>
        ))}
      </div>
    );
  };

  return (
    <>
      <p className={`${styles.line} ${styles.note}`}>
        <Info size={14} strokeWidth={2} aria-hidden />
        <span>{t("aiProfiles.weeklyNote")}</span>
      </p>

      <div className={`${styles.line} ${styles.autoRow}`}>
        <span className={styles.autoLabel}>{t("aiProfiles.autoLabel")}</span>
        <div className={styles.autoPicks}>
          {REPORTS.map((r) => (
            <span key={r} className={styles.autoPick}>
              <span className={styles.autoReport}>{t(`aiProfiles.feature.${r}`)}</span>
              <SimplePicker
                value={autoOf(r) ?? MAIN}
                options={autoOptions}
                onChange={(v) => commit(profiles, { ...auto, [r]: v === MAIN ? null : v })}
                size="md"
                title={t(`aiProfiles.feature.${r}`)}
              />
            </span>
          ))}
        </div>
      </div>

      <div className={`${styles.line} ${styles.profile}`}>
        <div className={styles.nameCol}>
          <span className={styles.mainName}>
            <Lock size={13} strokeWidth={2} aria-hidden />
            {t("aiProfiles.main")}
          </span>
          {autoBadges(null)}
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
              {autoBadges(id)}
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
