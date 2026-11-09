import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import {
  api,
  type SuperCategory,
  type SuperCategoryInput,
  type SuperCategoryPatch,
} from "../api/hindsight";
import { useCategories } from "./categories";
import { logError } from "../lib/logger";

interface SuperCategoriesContextValue {
  supers: SuperCategory[];
  loading: boolean;
  refresh: () => Promise<void>;
  create: (input: SuperCategoryInput) => Promise<SuperCategory>;
  update: (id: string, patch: SuperCategoryPatch) => Promise<void>;
  remove: (id: string) => Promise<void>;
  reorder: (orderedIds: string[]) => Promise<void>;
  /** 给某分类指派/移出大类。superId = null 表示移出回到"未归入"。 */
  assignCategory: (categoryId: string, superId: string | null) => Promise<void>;
  /** 跨大类移动：先改归属，再按目标落点写全局顺序，只刷新一次分类。 */
  moveCategoryToSuper: (
    categoryId: string,
    superId: string | null,
    orderedIds: string[],
  ) => Promise<void>;
}

const SuperCategoriesContext = createContext<SuperCategoriesContextValue | null>(
  null,
);

export function SuperCategoriesProvider({ children }: { children: ReactNode }) {
  const [supers, setSupers] = useState<SuperCategory[]>([]);
  const [loading, setLoading] = useState(true);
  // 操作分类的 super_category_id 后，要让 useCategories 也重拉一遍才能反映新归属
  const { refresh: refreshCategories } = useCategories();

  const refresh = useCallback(async () => {
    try {
      const list = await api.listSuperCategories();
      setSupers(list);
    } catch (e) {
      logError("superCategories.load", e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const create = useCallback(
    async (input: SuperCategoryInput) => {
      const created = await api.createSuperCategory(input);
      await refresh();
      return created;
    },
    [refresh],
  );

  const update = useCallback(
    async (id: string, patch: SuperCategoryPatch) => {
      await api.updateSuperCategory(id, patch);
      await refresh();
    },
    [refresh],
  );

  const remove = useCallback(
    async (id: string) => {
      await api.deleteSuperCategory(id);
      // 删大类会把子分类 super_category_id 置 NULL，所以 categories 也要重拉
      await Promise.all([refresh(), refreshCategories()]);
    },
    [refresh, refreshCategories],
  );

  const reorder = useCallback(
    async (orderedIds: string[]) => {
      // 乐观更新：先按新顺序刷一次本地 state，UI 立刻响应
      setSupers((prev) => {
        const idx = new Map(orderedIds.map((id, i) => [id, i]));
        return [...prev].sort((a, b) => {
          const ai = idx.get(a.id) ?? Number.MAX_SAFE_INTEGER;
          const bi = idx.get(b.id) ?? Number.MAX_SAFE_INTEGER;
          return ai - bi;
        });
      });
      await api.reorderSuperCategories(orderedIds);
      await refresh();
    },
    [refresh],
  );

  const assignCategory = useCallback(
    async (categoryId: string, superId: string | null) => {
      await api.assignCategoryToSuper(categoryId, superId);
      // 只刷新 categories（super_category_id 字段在 Category 上）
      await refreshCategories();
    },
    [refreshCategories],
  );

  const moveCategoryToSuper = useCallback(
    async (
      categoryId: string,
      superId: string | null,
      orderedIds: string[],
    ) => {
      // 先落归属，再落顺序。顺序失败时归属仍然正确，刷新后用户可以直接再拖一次。
      await api.assignCategoryToSuper(categoryId, superId);
      try {
        await api.reorderCategories(orderedIds);
      } finally {
        await refreshCategories();
      }
    },
    [refreshCategories],
  );

  const value = useMemo<SuperCategoriesContextValue>(
    () => ({
      supers,
      loading,
      refresh,
      create,
      update,
      remove,
      reorder,
      assignCategory,
      moveCategoryToSuper,
    }),
    [
      supers,
      loading,
      refresh,
      create,
      update,
      remove,
      reorder,
      assignCategory,
      moveCategoryToSuper,
    ],
  );

  return (
    <SuperCategoriesContext.Provider value={value}>
      {children}
    </SuperCategoriesContext.Provider>
  );
}

// eslint-disable-next-line react-refresh/only-export-components
export function useSuperCategories() {
  const ctx = useContext(SuperCategoriesContext);
  if (!ctx) {
    throw new Error(
      "useSuperCategories must be used within SuperCategoriesProvider",
    );
  }
  return ctx;
}
