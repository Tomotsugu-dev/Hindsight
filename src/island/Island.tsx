// The island page. Rust sends the content and says when the island opens. For each new content,
// the page measures the closed sides and the open panel, Rust answers where they go, and the page
// says when it is in place: only then does Rust show the window. See src-tauri/src/island/macos.rs.
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
  // Buttons stop their clicks before they get here
  useEffect(() => {
    const onClick = () => void invoke("island_clicked", { button: null });
    window.addEventListener("click", onClick);
    return () => window.removeEventListener("click", onClick);
  }, []);

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
      {drawn?.layout.left && drawn.content.closed.left ? (
        <Slot rect={drawn.layout.left}>
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
          className={styles.panel}
          style={{
            left: drawn.layout.open.x,
            width: drawn.layout.open.width,
            top: drawn.layout.topRight.height,
          }}
        >
          <Panel open={drawn.content.open} now={now} />
        </div>
      ) : null}
    </div>
  );
}
