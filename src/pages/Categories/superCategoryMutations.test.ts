import { describe, expect, it, vi } from "vitest";
import type { SuperCategory, SuperCategoryInput } from "../../api/hindsight";
import { demoteSuperTransaction, promoteCategoryTransaction } from "./superCategoryMutations";

const input: SuperCategoryInput = {
  name: "Focus",
  color: "#123456",
  icon: "Tag",
};

const created: SuperCategory = {
  id: "new",
  sortOrder: 1,
  ...input,
};

function promoteDeps(overrides: {
  createSuper?: () => Promise<SuperCategory>;
  assignCategory?: (categoryId: string, superId: string | null) => Promise<void>;
  reorderSupers?: (ids: string[]) => Promise<void>;
  deleteSuper?: (id: string) => Promise<void>;
}) {
  return {
    createSuper: vi.fn(overrides.createSuper ?? (() => Promise.resolve(created))),
    assignCategory: vi.fn(overrides.assignCategory ?? (() => Promise.resolve())),
    reorderSupers: vi.fn(overrides.reorderSupers ?? (() => Promise.resolve())),
    deleteSuper: vi.fn(overrides.deleteSuper ?? (() => Promise.resolve())),
  };
}

describe("promoteCategoryTransaction", () => {
  const request = {
    categoryId: "cat",
    sourceSuperId: "work",
    afterSuperId: "work",
    input,
    supers: ["work", "play"],
  };

  it("creates, assigns and inserts the new super in one transaction", async () => {
    const deps = promoteDeps({});
    await expect(promoteCategoryTransaction(request, deps)).resolves.toEqual({
      ok: true,
      value: created,
    });
    expect(deps.assignCategory).toHaveBeenCalledWith("cat", "new");
    expect(deps.reorderSupers).toHaveBeenCalledWith(["work", "new", "play"]);
    expect(deps.deleteSuper).not.toHaveBeenCalled();
  });

  it("restores the old ownership and super order when assignment fails", async () => {
    const deps = promoteDeps({
      assignCategory: (_categoryId, superId) => {
        if (superId === "new") {
          return Promise.reject(new Error("assign failed"));
        }
        return Promise.resolve();
      },
    });
    const result = await promoteCategoryTransaction(request, deps);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.rollback).toBe("complete");
    expect(deps.assignCategory).toHaveBeenLastCalledWith("cat", "work");
    expect(deps.reorderSupers).toHaveBeenLastCalledWith(["work", "play"]);
    expect(deps.deleteSuper).toHaveBeenCalledWith("new");
  });

  it("deletes the new super when the super reorder fails", async () => {
    let reorderCalls = 0;
    const deps = promoteDeps({
      reorderSupers: () => {
        reorderCalls += 1;
        if (reorderCalls === 1) {
          return Promise.reject(new Error("reorder failed"));
        }
        return Promise.resolve();
      },
    });
    const result = await promoteCategoryTransaction(request, deps);
    expect(result.ok).toBe(false);
    expect(deps.assignCategory).toHaveBeenLastCalledWith("cat", "work");
    expect(deps.reorderSupers).toHaveBeenLastCalledWith(["work", "play"]);
    expect(deps.deleteSuper).toHaveBeenCalledWith("new");
  });

  it("reports a partial rollback if deleting the new super fails", async () => {
    const deps = promoteDeps({
      assignCategory: (_categoryId, superId) => {
        if (superId === "new") {
          return Promise.reject(new Error("assign failed"));
        }
        return Promise.resolve();
      },
      deleteSuper: () => Promise.reject(new Error("delete failed")),
    });
    const result = await promoteCategoryTransaction(request, deps);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.rollback).toBe("partial");
  });

  it("does not mutate anything for a missing gap anchor", async () => {
    const deps = promoteDeps({});
    const result = await promoteCategoryTransaction({ ...request, afterSuperId: "missing" }, deps);
    expect(result.ok).toBe(false);
    expect(deps.createSuper).not.toHaveBeenCalled();
  });

  it("does not create anything when creating the new super fails", async () => {
    const deps = promoteDeps({
      createSuper: () => Promise.reject(new Error("create failed")),
    });
    const result = await promoteCategoryTransaction(request, deps);
    expect(result.ok).toBe(false);
    expect(deps.assignCategory).not.toHaveBeenCalled();
    expect(deps.reorderSupers).not.toHaveBeenCalled();
    expect(deps.deleteSuper).not.toHaveBeenCalled();
  });
});

function demoteDeps(overrides: {
  assignCategory?: (categoryId: string, superId: string | null) => Promise<void>;
  reorderCategories?: (ids: string[]) => Promise<void>;
  deleteSuper?: (id: string) => Promise<void>;
}) {
  return {
    assignCategory: vi.fn(overrides.assignCategory ?? (() => Promise.resolve())),
    reorderCategories: vi.fn(overrides.reorderCategories ?? (() => Promise.resolve())),
    deleteSuper: vi.fn(overrides.deleteSuper ?? (() => Promise.resolve())),
  };
}

describe("demoteSuperTransaction", () => {
  const request = {
    sourceSuperId: "work",
    targetSuperId: "play",
    categoryIds: ["a", "b"],
    orderedIds: ["p1", "a", "b", "p2"],
    originalCategoryOrder: ["a", "b", "p1", "p2"],
  };

  it("moves the block, writes its order and deletes the source", async () => {
    const deps = demoteDeps({});
    await expect(demoteSuperTransaction(request, deps)).resolves.toEqual({
      ok: true,
      value: undefined,
    });
    expect(deps.assignCategory.mock.calls).toEqual([
      ["a", "play"],
      ["b", "play"],
    ]);
    expect(deps.reorderCategories).toHaveBeenCalledWith(request.orderedIds);
    expect(deps.deleteSuper).toHaveBeenCalledWith("work");
  });

  it("restores already moved categories when a later assignment fails", async () => {
    const deps = demoteDeps({
      assignCategory: (categoryId, superId) => {
        if (categoryId === "b" && superId === "play") {
          return Promise.reject(new Error("assign failed"));
        }
        return Promise.resolve();
      },
    });
    const result = await demoteSuperTransaction(request, deps);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.rollback).toBe("complete");
    expect(deps.assignCategory).toHaveBeenLastCalledWith("a", "work");
    expect(deps.reorderCategories).not.toHaveBeenCalled();
    expect(deps.deleteSuper).not.toHaveBeenCalled();
  });

  it("restores category order and ownership when category reorder fails", async () => {
    const deps = demoteDeps({
      reorderCategories: () => Promise.reject(new Error("reorder failed")),
    });
    const result = await demoteSuperTransaction(request, deps);
    expect(result.ok).toBe(false);
    expect(deps.assignCategory).toHaveBeenCalledWith("b", "work");
    expect(deps.assignCategory).toHaveBeenCalledWith("a", "work");
    expect(deps.reorderCategories).toHaveBeenLastCalledWith(request.originalCategoryOrder);
    expect(deps.deleteSuper).not.toHaveBeenCalled();
  });

  it("deletes an empty source super without assigning any children", async () => {
    const deps = demoteDeps({});
    const result = await demoteSuperTransaction({ ...request, categoryIds: [] }, deps);
    expect(result).toEqual({ ok: true, value: undefined });
    expect(deps.assignCategory).not.toHaveBeenCalled();
    expect(deps.deleteSuper).toHaveBeenCalledWith("work");
  });

  it("restores ownership and order when deleting the source fails", async () => {
    const deps = demoteDeps({
      deleteSuper: () => Promise.reject(new Error("delete failed")),
    });
    const result = await demoteSuperTransaction(request, deps);
    expect(result.ok).toBe(false);
    expect(deps.assignCategory).toHaveBeenCalledWith("b", "work");
    expect(deps.assignCategory).toHaveBeenCalledWith("a", "work");
    expect(deps.reorderCategories).toHaveBeenLastCalledWith(request.originalCategoryOrder);
  });

  it("reports partial rollback when restoring a category fails", async () => {
    const deps = demoteDeps({
      assignCategory: (categoryId, superId) => {
        if (categoryId === "b" && superId === "play") {
          return Promise.reject(new Error("assign failed"));
        }
        if (categoryId === "a" && superId === "work") {
          return Promise.reject(new Error("rollback failed"));
        }
        return Promise.resolve();
      },
    });
    const result = await demoteSuperTransaction(request, deps);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.rollback).toBe("partial");
  });
});
