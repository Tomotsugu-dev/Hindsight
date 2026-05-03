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
      console.error("加载分类失败:", e);
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
    }),
    [categories, loading, getCategory, refresh, create, update, remove, assignApp, unassignApp],
  );

  return (
    <CategoriesContext.Provider value={value}>
      {children}
    </CategoriesContext.Provider>
  );
}

export function useCategories() {
  const ctx = useContext(CategoriesContext);
  if (!ctx) {
    throw new Error("useCategories must be used within CategoriesProvider");
  }
  return ctx;
}
