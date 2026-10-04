import { useEffect, useRef, type RefObject } from "react";

/** 每滚这么多像素算一格 */
const STEP_PX = 60;
/** 换层后这么久里的滚动全部吃掉：环正在变形（global.css 里 540ms），之后接着滚就接着算 */
const PAUSE_MS = 500;

/**
 * 在一个元素上用滚轮一格一格地操作。
 * - `canMove(down)` 为 false 时不拦截，页面照常滚，免得卡在这个元素上
 * - 每攒够 STEP_PX 调一次 `move(steps)`，steps 为正是往下；`move` 返回 true 表示换了一层，
 *   接下来 PAUSE_MS 里的滚动吃掉
 * - 刚挂上时（比如从另一层滚进来）同样先吃掉 PAUSE_MS
 *
 * React 的 onWheel 是 passive 的，preventDefault 不生效，所以挂原生监听。
 */
export function useWheelSteps(
  ref: RefObject<HTMLElement | null>,
  enabled: boolean,
  canMove: (down: boolean) => boolean,
  move: (steps: number) => boolean | void,
) {
  // 监听只在挂上时绑一次，回调每次渲染换成最新的
  const latest = useRef({ canMove, move });
  latest.current = { canMove, move };

  useEffect(() => {
    const el = ref.current;
    if (!enabled || !el) return;
    let acc = 0;
    let pauseUntil = performance.now() + PAUSE_MS;
    const onWheel = (e: WheelEvent) => {
      if (e.deltaY === 0) return;
      const now = performance.now();
      if (now < pauseUntil) {
        e.preventDefault();
        return;
      }
      const down = e.deltaY > 0;
      if (!latest.current.canMove(down)) return;
      e.preventDefault();
      // 换方向时重新累计
      if (acc !== 0 && acc > 0 !== down) acc = 0;
      acc += e.deltaY;
      const steps = Math.trunc(acc / STEP_PX);
      if (steps === 0) return;
      acc -= steps * STEP_PX;
      if (latest.current.move(steps)) {
        acc = 0;
        pauseUntil = now + PAUSE_MS;
      }
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [ref, enabled]);
}
