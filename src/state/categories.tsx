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
  type Category,
  type CategoryInput,
  type CategoryPatch,
} from "../api/hindsight";
import { logError } from "../lib/logger";

interface CategoriesContextValue {
  categories: Category[];
  loading: boolean;
  getCategory: (id: string) => Category | undefined;
  refresh: () => Promise<void>;
  create: (input: CategoryInput) => Promise<Category>;
  update: (id: string, patch: CategoryPatch) => Promise<void>;
  remove: (id: string) => Promise<void>;
  assignApp: (processName: string, categoryId: string) => Promise<void>;
  unassignApp: (processName: string) => Promise<void>;
  reorder: (orderedIds: string[]) => Promise<void>;
}

const CategoriesContext = createContext<CategoriesContextValue | null>(null);

export function CategoriesProvider({ children }: { children: ReactNode }) {
  const [categories, setCategories] = useState<Category[]>([]);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    try {
      const list = await api.listCategories();
      setCategories(list);
    } catch (e) {
      logError("categories.load", e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const create = useCallback(
    async (input: CategoryInput) => {
      const created = await api.createCategory(input);
      await refresh();
      return created;
    },
    [refresh],
  );

  const update = useCallback(
    async (id: string, patch: CategoryPatch) => {
      await api.updateCategory(id, patch);
      await refresh();
    },
    [refresh],
  );

  const remove = useCallback(
    async (id: string) => {
      await api.deleteCategory(id);
      await refresh();
    },
    [refresh],
  );

  const assignApp = useCallback(
    async (processName: string, categoryId: string) => {
      await api.assignApp(processName, categoryId);
      await refresh();
    },
    [refresh],
  );

  const unassignApp = useCallback(
    async (processName: string) => {
      await api.unassignApp(processName);
      await refresh();
    },
    [refresh],
  );

  const reorder = useCallback(
    async (orderedIds: string[]) => {
      // 乐观更新：先按新顺序刷一次本地 state，让 UI 立刻响应；同时调后端
      // 持久化 + 推同步。后端落库后 refresh() 会拿到权威顺序覆盖。
      setCategories((prev) => {
        const idx = new Map(orderedIds.map((id, i) => [id, i]));
        return [...prev].sort((a, b) => {
          const ai = idx.get(a.id) ?? Number.MAX_SAFE_INTEGER;
          const bi = idx.get(b.id) ?? Number.MAX_SAFE_INTEGER;
          return ai - bi;
        });
      });
      await api.reorderCategories(orderedIds);
      await refresh();
    },
    [refresh],
  );

  const getCategory = useCallback(
    (id: string) => categories.find((c) => c.id === id),
    [categories],
  );

  const value = useMemo<CategoriesContextValue>(
    () => ({
      categories,
      loading,
      getCategory,
      refresh,
      create,
      update,
      remove,
      assignApp,
      unassignApp,
      reorder,
    }),
    [categories, loading, getCategory, refresh, create, update, remove, assignApp, unassignApp, reorder],
  );

  return (
    <CategoriesContext.Provider value={value}>
      {children}
    </CategoriesContext.Provider>
  );
}

// hook 跟 Provider 同文件是有意为之——消费方一次 import 解决，dev 期 fast refresh
// 在改动 Provider 时退化为整页刷新（state 文件极少改动，影响可接受）。
// eslint-disable-next-line react-refresh/only-export-components
export function useCategories() {
  const ctx = useContext(CategoriesContext);
  if (!ctx) {
    throw new Error("useCategories must be used within CategoriesProvider");
  }
  return ctx;
}
