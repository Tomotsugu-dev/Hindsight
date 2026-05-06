import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { Plus, Trash2, X } from "lucide-react";
import { api, type AppGroup, type AppGroupMember } from "../../api/hindsight";
import { AppIcon } from "../../components/AppIcon/AppIcon";
import { useCategories } from "../../state/categories";
import { useDeviceFilter, type Device } from "../../state/deviceFilter";
import { logError } from "../../lib/logger";
import { displayAppName } from "../../utils/displayName";
import { AssignDropdown } from "./parts";
import styles from "./Pairing.module.css";

/** 把 group + 设备列表换算成「每个 device 列对应哪个 member（如果有）」的 lookup */
function membersByDevice(group: AppGroup, devices: Device[]): (AppGroupMember | null)[] {
  return devices.map((d) => group.members.find((m) => m.lastDeviceId === d.id) ?? null);
}

function fmtDuration(secs: number): string {
  if (secs < 60) return `${secs}s`;
  const m = Math.round(secs / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  const rem = m % 60;
  return rem === 0 ? `${h}h` : `${h}h${rem}m`;
}

/** 拖拽状态：源 process / 源组 / 锁定的列索引 / 源列水平中心（屏幕坐标）/ 当前 cursor Y */
interface DragState {
  processName: string;
  sourceGroupId: string;
  deviceColIdx: number;
  /** 源列的水平中心 X，固定，飞行 chip 锁在这条线上（实现「列内移动」） */
  lockedX: number;
  /** 飞行 chip 显示用的初始内容快照（避免 setState race） */
  displayName: string;
  recentSecs: number;
  /** 当前鼠标 Y（屏幕坐标） */
  cursorY: number;
}

export function PairingSection() {
  const { t } = useTranslation();
  const { devices } = useDeviceFilter();
  const { categories, refresh: refreshCategories } = useCategories();
  const [groups, setGroups] = useState<AppGroup[] | null>(null);
  const [drag, setDrag] = useState<DragState | null>(null);
  const [hoverGroupId, setHoverGroupId] = useState<string | null>(null);
  const [pendingNames, setPendingNames] = useState<Record<string, string>>({});

  // 每行（一个 group）的 DOM 引用，mousemove 时拿来做命中检测
  const rowRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const setRowRef = useCallback(
    (id: string) => (el: HTMLDivElement | null) => {
      if (el) rowRefs.current.set(id, el);
      else rowRefs.current.delete(id);
    },
    [],
  );

  const reload = async () => {
    try {
      const list = await api.listAppGroups();
      setGroups(list);
    } catch (e) {
      logError("pairing.listGroups", e);
      setGroups([]);
    }
  };

  useEffect(() => {
    void reload();
  }, []);

  // self 设备先排前面，其它按名字稳定排序
  const sortedDevices = useMemo<Device[]>(() => {
    const arr = [...devices];
    arr.sort((a, b) => {
      if (a.current && !b.current) return -1;
      if (!a.current && b.current) return 1;
      return a.name.localeCompare(b.name);
    });
    return arr;
  }, [devices]);

  // 显示规则：
  //   - 空组保留（误操作 merge 后源行变空 → 用户能看到、能拖回）
  //   - 已删的组不显示（list_groups 已过滤 deleted_at IS NULL，保险再过一遍）
  //   - 有成员但全在未知设备上的也保留（避免数据偶尔漏 device_meta 时整行消失）
  const visibleGroups = useMemo(() => {
    if (!groups) return null;
    return groups;
  }, [groups]);

  // —— 拖拽：mousemove 实时锁 X、跟随 Y、命中检测 ——
  useEffect(() => {
    if (!drag) return;
    const onMove = (e: MouseEvent) => {
      setDrag((d) => (d ? { ...d, cursorY: e.clientY } : null));
      // 命中检测：找鼠标 Y 落在哪个 row 内（X 不参与，因为我们要求列锁但仍允许拖到任意行）
      let hit: string | null = null;
      for (const [gid, el] of rowRefs.current) {
        const r = el.getBoundingClientRect();
        if (e.clientY >= r.top && e.clientY <= r.bottom) {
          hit = gid;
          break;
        }
      }
      setHoverGroupId(hit);
    };
    const onUp = () => {
      const cur = drag;
      const target = hoverGroupId;
      setDrag(null);
      setHoverGroupId(null);
      if (cur && target && target !== cur.sourceGroupId) {
        void api
          .mergeAppGroup(cur.processName, target)
          .then(() => Promise.all([reload(), refreshCategories()]))
          .catch((e) => logError("pairing.merge", e));
      }
    };
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
    return () => {
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
    };
  }, [drag, hoverGroupId, refreshCategories]);

  if (visibleGroups === null) {
    return <div className={styles.toolbar}>{t("categories.pairing.loading")}</div>;
  }
  if (sortedDevices.length === 0) {
    return <div className={styles.toolbar}>{t("categories.pairing.noDevices")}</div>;
  }

  // 设备列每个 minmax(160px, 1fr)：在容器够宽时弹性 1fr 分配；容器不够宽时
  // 缩到最小 160px 后触发表格容器的横向滚动（多设备场景下窗口装不下也不挤压）
  const deviceColsTemplate = sortedDevices
    .map(() => "minmax(160px, 1fr)")
    .join(" ");
  const cssVars = { "--device-cols": deviceColsTemplate } as CSSProperties;

  const startDrag = (
    e: React.MouseEvent<HTMLDivElement>,
    member: AppGroupMember,
    sourceGroupId: string,
    deviceColIdx: number,
  ) => {
    if (e.button !== 0) return; // 只接左键
    e.preventDefault();
    // 用源 chip 的列容器水平中心做 lockedX —— 后续飞行 chip 永远停在这条线上
    const colEl = e.currentTarget as HTMLElement;
    const rect = colEl.getBoundingClientRect();
    const lockedX = rect.left + rect.width / 2;
    setDrag({
      processName: member.processName,
      sourceGroupId,
      deviceColIdx,
      lockedX,
      displayName: displayAppName(member.processName),
      recentSecs: member.recentSecs,
      cursorY: e.clientY,
    });
  };

  const onDeleteRow = async (groupId: string) => {
    try {
      await api.deleteAppGroup(groupId);
      await reload();
    } catch (e) {
      logError("pairing.deleteRow", e);
    }
  };

  const onCreateGroup = async () => {
    // 默认名 + 时间后缀防重；用户可立即在 nameInput 里改
    const time = new Date().toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
    });
    const defaultName = t("categories.pairing.newRowDefaultName", { time });
    try {
      await api.createAppGroup(defaultName);
      await reload();
    } catch (e) {
      logError("pairing.createRow", e);
    }
  };

  const onUnmerge = async (processName: string) => {
    try {
      await api.unmergeAppGroup(processName);
      await Promise.all([reload(), refreshCategories()]);
    } catch (e) {
      logError("pairing.unmerge", e);
    }
  };

  const onAssignCategory = async (groupId: string, categoryId: string | null) => {
    try {
      await api.assignAppGroupCategory(groupId, categoryId);
      await Promise.all([reload(), refreshCategories()]);
    } catch (e) {
      logError("pairing.assignCategory", e);
    }
  };

  const onCommitName = async (groupId: string) => {
    const next = pendingNames[groupId];
    if (next === undefined) return;
    const trimmed = next.trim();
    if (!trimmed) {
      setPendingNames((p) => {
        const cp = { ...p };
        delete cp[groupId];
        return cp;
      });
      return;
    }
    try {
      await api.renameAppGroup(groupId, trimmed);
      await reload();
    } catch (e) {
      logError("pairing.rename", e);
    } finally {
      setPendingNames((p) => {
        const cp = { ...p };
        delete cp[groupId];
        return cp;
      });
    }
  };

  return (
    <div className={styles.pairing} style={cssVars}>
      <div className={styles.devHeader}>
        {sortedDevices.map((d) => (
          <span key={d.id} className={styles.devHeaderName}>
            {d.name}
          </span>
        ))}
        <span className={styles.devHeaderName}>
          {t("categories.pairing.header.displayName")}
        </span>
        <span className={styles.devHeaderActionPad} />
        <span className={styles.deleteCol} />
      </div>

      {visibleGroups.map((group, idx) => {
        const slots = membersByDevice(group, sortedDevices);
        const isPaired = group.members.length > 1;
        const isHot = drag !== null && hoverGroupId === group.id && drag.sourceGroupId !== group.id;
        const isSourceRow = drag !== null && drag.sourceGroupId === group.id;
        const nameDraft = pendingNames[group.id] ?? group.displayName;

        // 没分类的行用淡黄橙色高亮，提醒用户去指派；
        // 注意还要排除找不到对应 active category 的孤儿引用情况（categoryId 非 null 但 categories 里没匹配）
        const hasActiveCategory =
          group.categoryId != null &&
          categories.some((c) => c.id === group.categoryId);
        return (
          <div
            key={group.id}
            ref={setRowRef(group.id)}
            className={[
              styles.row,
              idx % 2 === 0 ? styles.rowEven : styles.rowOdd,
              isHot ? styles.hot : "",
              isSourceRow ? styles.sourceRow : "",
              !hasActiveCategory ? styles.unassigned : "",
            ]
              .filter(Boolean)
              .join(" ")}
          >
            {slots.map((member, idx) => {
              const dev = sortedDevices[idx];
              if (!member) {
                return (
                  <div key={dev.id} className={`${styles.devCol} ${styles.empty}`}>
                    <span className={styles.emptyDash} aria-hidden />
                  </div>
                );
              }
              const isDraggingThis =
                drag !== null && drag.processName === member.processName;
              return (
                <div
                  key={dev.id}
                  className={styles.devCol}
                  onMouseDown={(e) => startDrag(e, member, group.id, idx)}
                >
                  <span
                    className={`${styles.chip} ${
                      isDraggingThis ? styles.chipPlaceholder : ""
                    }`}
                  >
                    <AppIcon
                      processName={member.processName}
                      fallbackColor="#94a3b8"
                      size={14}
                    />
                    <span className={styles.chipName} title={member.processName}>
                      {displayAppName(member.processName)}
                    </span>
                    <span className={styles.chipMeta}>{fmtDuration(member.recentSecs)}</span>
                    {isPaired && !isDraggingThis && (
                      <button
                        type="button"
                        className={styles.chipUnmerge}
                        onMouseDown={(e) => e.stopPropagation()}
                        onClick={() => void onUnmerge(member.processName)}
                        title={t("categories.pairing.chip.unmergeTooltip")}
                      >
                        <X size={11} strokeWidth={2.25} />
                      </button>
                    )}
                  </span>
                </div>
              );
            })}

            <div className={styles.nameCol}>
              <input
                className={styles.nameInput}
                value={nameDraft}
                onMouseDown={(e) => e.stopPropagation()}
                onChange={(e) =>
                  setPendingNames((p) => ({ ...p, [group.id]: e.target.value }))
                }
                onBlur={() => void onCommitName(group.id)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.currentTarget.blur();
                  } else if (e.key === "Escape") {
                    setPendingNames((p) => {
                      const cp = { ...p };
                      delete cp[group.id];
                      return cp;
                    });
                    e.currentTarget.blur();
                  }
                }}
              />
            </div>

            <div className={styles.actionCol}>
              <AssignDropdown
                categories={categories}
                currentCategoryId={group.categoryId}
                allowClear
                onPick={(cid) => void onAssignCategory(group.id, cid)}
              />
            </div>

            <div className={styles.deleteCol}>
              {group.members.length === 0 && (
                <button
                  type="button"
                  className={styles.deleteRowBtn}
                  onMouseDown={(e) => e.stopPropagation()}
                  onClick={() => void onDeleteRow(group.id)}
                  title={t("categories.pairing.row.deleteEmptyTooltip")}
                  aria-label={t("categories.pairing.row.deleteEmptyAria")}
                >
                  <Trash2 size={12} strokeWidth={2.25} />
                </button>
              )}
            </div>
          </div>
        );
      })}

      <button
        type="button"
        className={styles.createRowBtn}
        onClick={() => void onCreateGroup()}
      >
        <Plus size={12} strokeWidth={2.25} />
        {t("categories.pairing.newRow")}
      </button>

      {drag &&
        createPortal(
          <div
            className={styles.flyChip}
            style={{
              // X 锁在源列水平中心；Y 跟着鼠标
              left: drag.lockedX,
              top: drag.cursorY,
            }}
          >
            <AppIcon processName={drag.processName} fallbackColor="#94a3b8" size={14} />
            <span>{drag.displayName}</span>
            <span className={styles.flyChipMeta}>{fmtDuration(drag.recentSecs)}</span>
          </div>,
          document.body,
        )}
    </div>
  );
}
