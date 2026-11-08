import { useCallback, useRef, useState } from "react";
import { api, type DataCount, type PurgeTarget } from "../../api/hindsight";
import { errorText, purgeFailedReason } from "../../utils/privateCategory";

/** 统计会删多少：还在统计、统计好了、统计出错。 */
export type CountState =
  | { kind: "loading" }
  | { kind: "done"; count: DataCount }
  | { kind: "failed"; error: string };

/** 弹窗走到了哪一步。 */
export type PrivacyStep =
  /** 问用户要不要设为隐私 */
  | { kind: "ask"; count: CountState }
  /** 正在保存、删除，关不掉 */
  | { kind: "deleting" }
  /** 设置没存上 */
  | { kind: "saveFailed"; error: string }
  /** 设置存好了，旧数据没删完 */
  | { kind: "purgeFailed"; error: string };

export interface PrivacyDialogState {
  /** 应用名或域名。null：合并应用、删网站规则以后删除失败，不是在设置某一个 */
  name: string | null;
  site: boolean;
  /** 保存设置的命令，后端存好以后接着删数据。null：只能重试删除 */
  save: (() => Promise<void>) | null;
  step: PrivacyStep;
}

/** 要设为隐私的应用或网站。 */
export interface PrivacyRequest {
  name: string;
  site: boolean;
  /** 统计会删多少用 */
  target: PurgeTarget;
  save: () => Promise<void>;
}

/**
 * 「设为隐私」弹窗的状态和操作。返回的 `dialogProps` 交给 `PrivacyDialog`。
 *
 * `onSaved` 在设置存好以后调，删除失败也调：设置已经生效，调用方要刷新列表。
 */
export function usePrivacyDialog(onSaved: () => void | Promise<unknown>) {
  const [state, setState] = useState<PrivacyDialogState | null>(null);
  // 每打开一次加一。关掉或换了一个以后才回来的结果，按它丢掉
  const openId = useRef(0);

  const show = (next: PrivacyDialogState) => {
    openId.current += 1;
    setState(next);
    return openId.current;
  };
  const setStep = (id: number, step: PrivacyStep) =>
    setState((s) => (s && openId.current === id ? { ...s, step } : s));
  const closeIf = (id: number) => setState((s) => (openId.current === id ? null : s));

  const save = async (id: number, run: () => Promise<void>) => {
    setStep(id, { kind: "deleting" });
    try {
      await run();
      void onSaved();
      closeIf(id);
    } catch (e) {
      const reason = purgeFailedReason(e);
      if (reason === null) {
        setStep(id, { kind: "saveFailed", error: errorText(e) });
      } else {
        void onSaved();
        setStep(id, { kind: "purgeFailed", error: reason });
      }
    }
  };

  const purge = async (id: number) => {
    setStep(id, { kind: "deleting" });
    try {
      await api.purgePrivateData();
      closeIf(id);
    } catch (e) {
      setStep(id, { kind: "purgeFailed", error: errorText(e) });
    }
  };

  /** 先问。问的时候统计会删多少。 */
  const confirm = (req: PrivacyRequest) => {
    const { target, ...rest } = req;
    const id = show({ ...rest, step: { kind: "ask", count: { kind: "loading" } } });
    const setCount = (count: CountState) =>
      setState((s) =>
        s && openId.current === id && s.step.kind === "ask"
          ? { ...s, step: { kind: "ask", count } }
          : s,
      );
    void api.countTargetData(target).then(
      (count) => setCount({ kind: "done", count }),
      (e) => setCount({ kind: "failed", error: errorText(e) }),
    );
  };

  /** 不问，直接存：用户已经在别的弹窗里选过了。 */
  const run = (req: { name: string; save: () => Promise<void> }) => {
    const id = show({ ...req, site: false, step: { kind: "deleting" } });
    void save(id, req.save);
  };

  /** 合并应用、删网站规则以后删除失败：设置已经存好了，只能重试删除。 */
  const showPurgeFailed = useCallback((error: string) => {
    openId.current += 1;
    setState({ name: null, site: false, save: null, step: { kind: "purgeFailed", error } });
  }, []);

  const onConfirm = () => {
    if (!state || state.step.kind === "deleting") return;
    const id = openId.current;
    if (state.step.kind === "purgeFailed" || state.save === null) void purge(id);
    else void save(id, state.save);
  };

  const onCancel = () => {
    if (state?.step.kind === "deleting") return;
    openId.current += 1;
    setState(null);
  };

  return { confirm, run, showPurgeFailed, dialogProps: { state, onConfirm, onCancel } };
}
