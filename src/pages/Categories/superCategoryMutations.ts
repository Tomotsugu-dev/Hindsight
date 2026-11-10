import type { SuperCategory, SuperCategoryInput } from "../../api/hindsight";
import { insertSuperAfter } from "./dragOrder";

export type DragMutationResult<T> =
  | { ok: true; value: T }
  | {
      ok: false;
      rollback: "complete" | "partial";
      error: unknown;
    };

export interface PromoteCategoryInput {
  categoryId: string;
  sourceSuperId: string | null;
  afterSuperId: string;
  input: SuperCategoryInput;
  supers: readonly string[];
}

export interface PromoteCategoryDeps {
  createSuper: (input: SuperCategoryInput) => Promise<SuperCategory>;
  assignCategory: (categoryId: string, superId: string | null) => Promise<void>;
  reorderSupers: (orderedIds: string[]) => Promise<void>;
  deleteSuper: (superId: string) => Promise<void>;
}

export async function promoteCategoryTransaction(
  request: PromoteCategoryInput,
  deps: PromoteCategoryDeps,
): Promise<DragMutationResult<SuperCategory>> {
  if (!request.supers.includes(request.afterSuperId)) {
    return {
      ok: false,
      rollback: "complete",
      error: new Error("promote target gap no longer exists"),
    };
  }

  let created: SuperCategory | null = null;
  try {
    created = await deps.createSuper(request.input);
    const nextOrder = insertSuperAfter(request.supers, request.afterSuperId, created.id);
    if (!nextOrder) throw new Error("promote target gap no longer exists");

    await deps.assignCategory(request.categoryId, created.id);
    await deps.reorderSupers(nextOrder);
    return { ok: true, value: created };
  } catch (error) {
    if (!created) {
      return { ok: false, rollback: "complete", error };
    }

    let complete = true;
    let restoredCategory = false;
    try {
      await deps.assignCategory(request.categoryId, request.sourceSuperId);
      restoredCategory = true;
    } catch {
      complete = false;
    }

    try {
      await deps.reorderSupers([...request.supers]);
    } catch {
      complete = false;
    }

    // 分类已成功还原则可安全删除刚建的大类；否则保留它，避免分类被
    // 后端“删大类 = 子分类置空”的级联带到错误位置。
    if (restoredCategory) {
      try {
        await deps.deleteSuper(created.id);
      } catch {
        complete = false;
      }
    }

    return {
      ok: false,
      rollback: complete ? "complete" : "partial",
      error,
    };
  }
}

export interface DemoteSuperInput {
  sourceSuperId: string;
  targetSuperId: string;
  categoryIds: readonly string[];
  orderedIds: readonly string[];
  originalCategoryOrder: readonly string[];
}

export interface DemoteSuperDeps {
  assignCategory: (categoryId: string, superId: string | null) => Promise<void>;
  reorderCategories: (orderedIds: string[]) => Promise<void>;
  deleteSuper: (superId: string) => Promise<void>;
}

export async function demoteSuperTransaction(
  request: DemoteSuperInput,
  deps: DemoteSuperDeps,
): Promise<DragMutationResult<void>> {
  if (request.sourceSuperId === request.targetSuperId) {
    return {
      ok: false,
      rollback: "complete",
      error: new Error("cannot demote a super-category into itself"),
    };
  }

  const moved: string[] = [];
  let reorderAttempted = false;
  try {
    for (const categoryId of request.categoryIds) {
      await deps.assignCategory(categoryId, request.targetSuperId);
      moved.push(categoryId);
    }

    reorderAttempted = true;
    await deps.reorderCategories([...request.orderedIds]);
    await deps.deleteSuper(request.sourceSuperId);
    return { ok: true, value: undefined };
  } catch (error) {
    let complete = true;
    for (const categoryId of [...moved].reverse()) {
      try {
        await deps.assignCategory(categoryId, request.sourceSuperId);
      } catch {
        complete = false;
      }
    }

    if (reorderAttempted) {
      try {
        await deps.reorderCategories([...request.originalCategoryOrder]);
      } catch {
        complete = false;
      }
    }

    return {
      ok: false,
      rollback: complete ? "complete" : "partial",
      error,
    };
  }
}
