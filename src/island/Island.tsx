// The island page. Rust sends the content and says when the island opens. For each new content,
// the page measures the closed sides and the open panel, Rust answers where they go, and the page
// says when it is in place: only then does Rust show the window. See src-tauri/src/island/macos/.
// On Windows (src-tauri/src/island/windows/) the window is clear: the page draws the capsule and
// the open island itself, tells Rust when the mouse comes in and leaves, and when to drag.
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Block, Side, TextView } from "./parts";
import type { Component, Content, Layout, PageRect } from "./types";
import styles from "./Island.module.css";

/** Space on each side of the closed content in a capsule */
const CAPSULE_PADDING = 10;
/** 右上角的字两边留的空，和 CSS 里 .topRight 的一样 */
const TOP_RIGHT_PADDING = 12;
/** 展开的岛的圆角，和 Rust 的 OPEN_RADIUS 一样。Windows 上页面自己画形状时用 */
const OPEN_RADIUS = 16;
/** 鼠标停多久才展开（Windows）：想拖动时一碰不会弹开 */
const HOVER_DELAY = 150;
/** 按下以后光标在屏幕上移动超过这么多才算拖动，不算点击。和 Rust 那边一样 */
const DRAG_SLOP = 4;

/** A content with where Rust put its sides */
type Placed = { content: Content; layout: Layout };

type Open = NonNullable<Content["open"]>;

/** The width a closed side asks for. `null`: nothing on that side. */
function widthOf(element: HTMLElement | null, component: Component | null): number | null {
  if (!component || !element) return null;
  // An icon, or a ring with no text, sits in a circle: Rust makes a capsule at least as wide as
  // it is tall
  if (component.type === "icon" || (component.type === "ring" && !component.text)) return 0;
  return element.getBoundingClientRect().width + 2 * CAPSULE_PADDING;
}

/** The time now, every half second, for the timers. */
function useNow(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 500);
    return () => window.clearInterval(id);
  }, []);
  return now;
}

type SlotProps = { rect: PageRect; className?: string; children: ReactNode };

/** 页面自己画的形状（Windows）：放在 `rect`，圆角 `radius` */
function shapeStyle({ x, y, width, height }: PageRect, radius: number) {
  return { left: x, top: y, width, height, borderRadius: radius };
}

function Slot({ rect, className, children }: SlotProps) {
  const { x, y, width, height } = rect;
  return (
    <div
      className={className ? `${styles.slot} ${className}` : styles.slot}
      style={{ left: x, top: y, width, height }}
    >
      {children}
    </div>
  );
}

/** 展开时下面的面板：左边一列；右边一行，下面可以再有一个小字。量高度的那份和显示的那份是同一个
 * 组件 */
function Panel({ open, now }: { open: Open; now: number }) {
  return (
    <div className={styles.panelBody}>
      <div className={styles.column}>
        {open.left.map((component, i) => (
          <Block key={i} component={component} now={now} />
        ))}
      </div>
      <div className={styles.right}>
        <div className={styles.row}>
          {open.right.map((component, i) => (
            <Block key={i} component={component} now={now} />
          ))}
        </div>
        {open.bottomRight ? <TextView text={open.bottomRight} /> : null}
      </div>
    </div>
  );
}

export function Island() {
  const [content, setContent] = useState<Content | null>(null);
  // What is drawn. A new content is drawn only once Rust has placed it, so it never shows in the
  // places of the one before
  const [placed, setPlaced] = useState<Placed | null>(null);
  const [open, setOpen] = useState(false);
  // Windows：页面自己画形状
  const shaped = Boolean(placed?.layout.drawn);
  const now = useNow();
  const left = useRef<HTMLSpanElement>(null);
  const right = useRef<HTMLSpanElement>(null);
  const topRightText = useRef<HTMLSpanElement>(null);
  const panel = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let changed = false;
    const stops = [
      listen<Content | null>("island-content", ({ payload }) => {
        changed = true;
        setContent(payload);
      }),
      listen<boolean>("island-open", ({ payload }) => setOpen(payload)),
      listen<Layout>("island-layout", ({ payload }) =>
        setPlaced((p) => p && { ...p, layout: payload }),
      ),
    ];
    // Listen before asking, so a change in between is not missed
    void Promise.all(stops)
      .then(() => invoke<Content | null>("island_content"))
      .then((c) => {
        if (!changed) setContent(c);
      });
    return () => {
      for (const stop of stops) void stop.then((unlisten) => unlisten());
    };
  }, []);

  // The whole window is the island: a click anywhere but on a button opens the plugin's page.
  // Buttons stop their clicks before they get here. On Windows, pressing anywhere but on a button
  // drags the island, and letting go after a drag is not a click
  useEffect(() => {
    let pressed: { x: number; y: number } | null = null;
    const onDown = (e: PointerEvent) => {
      pressed = { x: e.screenX, y: e.screenY };
      const onButton = e.target instanceof Element && e.target.closest("button");
      if (shaped && e.button === 0 && !onButton) void invoke("island_drag");
    };
    const onClick = (e: MouseEvent) => {
      if (pressed && Math.hypot(e.screenX - pressed.x, e.screenY - pressed.y) > DRAG_SLOP) return;
      void invoke("island_clicked", { button: null });
    };
    window.addEventListener("pointerdown", onDown);
    window.addEventListener("click", onClick);
    return () => {
      window.removeEventListener("pointerdown", onDown);
      window.removeEventListener("click", onClick);
    };
  }, [shaped]);

  // Windows：窗口是透明的，页面只画胶囊和岛。macOS 整页是黑的，形状由遮罩裁出来
  useEffect(() => {
    if (!shaped) return;
    document.documentElement.style.background = "transparent";
    document.body.style.background = "transparent";
  }, [shaped]);

  // Windows：鼠标移入移出由页面告诉 Rust。停一会儿才展开
  useEffect(() => {
    if (!shaped) return;
    let inside = false;
    let timer: number | undefined;
    const enter = () => {
      if (inside) return;
      inside = true;
      timer = window.setTimeout(() => void invoke("island_hover", { inside: true }), HOVER_DELAY);
    };
    // relatedTarget 为空：鼠标离开了窗口露出来的那部分
    const leave = (e: MouseEvent) => {
      if (e.relatedTarget || !inside) return;
      inside = false;
      window.clearTimeout(timer);
      void invoke("island_hover", { inside: false });
    };
    document.addEventListener("mouseover", enter);
    document.addEventListener("mouseout", leave);
    return () => {
      window.clearTimeout(timer);
      document.removeEventListener("mouseover", enter);
      document.removeEventListener("mouseout", leave);
    };
  }, [shaped]);

  // Measures the closed sides and the open panel of a new content, and draws it where Rust puts
  // them
  useLayoutEffect(() => {
    if (!content) return;
    let current = true;
    const opened = () => {
      const box = panel.current?.getBoundingClientRect();
      if (!content.open || !box) return null;
      const text = content.open.topRight ? topRightText.current?.getBoundingClientRect() : null;
      return {
        width: box.width,
        height: box.height,
        topRight: text ? text.width + 2 * TOP_RIGHT_PADDING : null,
      };
    };
    void document.fonts.ready
      .then(() =>
        invoke<Layout | null>("island_measured", {
          left: widthOf(left.current, content.closed.left),
          right: widthOf(right.current, content.closed.right),
          panel: opened(),
        }),
      )
      .then((layout) => {
        if (current && layout) setPlaced({ content, layout });
      });
    return () => {
      current = false;
    };
  }, [content]);

  // The content is in place once React has updated the page. Not requestAnimationFrame: a hidden
  // window does not run it, and the window shows only after this call
  useEffect(() => {
    if (placed) void invoke("island_ready");
  }, [placed]);

  const drawn = content ? placed : null;
  const topRight = drawn?.content.open?.topRight;
  // Windows：页面自己画的形状。展开时左边的内容贴着展开的岛的左边
  const shape = drawn?.layout.drawn;
  const leftRect = open && shape?.openLeft ? shape.openLeft : drawn?.layout.left;

  return (
    <div className={open ? `${styles.island} ${styles.open}` : styles.island}>
      {/* A timer is measured once: it only gets narrower as it counts down */}
      <div className={styles.measure} aria-hidden>
        <span ref={left}>
          {content?.closed.left ? <Side component={content.closed.left} now={now} /> : null}
        </span>
        <span ref={right}>
          {content?.closed.right ? <Side component={content.closed.right} now={now} /> : null}
        </span>
        <span ref={topRightText}>
          {content?.open?.topRight ? <TextView text={content.open.topRight} /> : null}
        </span>
        {/* 不折行时多宽多高，最宽到窗口那么宽 */}
        <div ref={panel} className={styles.measurePanel}>
          {content?.open ? <Panel open={content.open} now={now} /> : null}
        </div>
      </div>
      {drawn && shape ? (
        <div
          className={styles.shape}
          style={
            open
              ? shapeStyle(drawn.layout.open, OPEN_RADIUS)
              : shapeStyle(shape.capsule, shape.capsule.height / 2)
          }
        />
      ) : null}
      {leftRect && drawn?.content.closed.left ? (
        <Slot rect={leftRect} className={shape ? styles.follows : undefined}>
          <Side component={drawn.content.closed.left} now={now} />
        </Slot>
      ) : null}
      {drawn?.layout.right && drawn.content.closed.right ? (
        // Open, the text at the top right takes its place
        <Slot rect={drawn.layout.right} className={topRight ? styles.closedOnly : undefined}>
          <Side component={drawn.content.closed.right} now={now} />
        </Slot>
      ) : null}
      {drawn && topRight ? (
        <Slot rect={drawn.layout.topRight} className={styles.topRight}>
          <TextView text={topRight} />
        </Slot>
      ) : null}
      {drawn?.content.open ? (
        <div
          className={shape?.up ? `${styles.panel} ${styles.up}` : styles.panel}
          style={{
            left: drawn.layout.open.x,
            width: drawn.layout.open.width,
            // 面板在最上面那一条下面；往上展开时在岛的最上面
            top: shape?.up
              ? drawn.layout.open.y
              : drawn.layout.topRight.y + drawn.layout.topRight.height,
          }}
        >
          <Panel open={drawn.content.open} now={now} />
        </div>
      ) : null}
    </div>
  );
}
