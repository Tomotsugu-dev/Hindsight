import { useEffect, useRef, useState } from "react";
import type { LucideIcon } from "lucide-react";
import { activeSection } from "./activeSection";
import styles from "./PageToc.module.css";

export interface PageTocItem {
  /** The id of the element the item jumps to */
  id: string;
  label: string;
  icon: LucideIcon;
}

/** How far below the top of the scroll area a section counts as reached, in px. It is below the floating tab bar. */
const READ_LINE = 80;

/**
 * Shows a table of contents for one long page.
 *
 * It stays in view while the page scrolls, marks the section being read, and jumps to a section on
 * click. The page gives each section the id of its item.
 */
export function PageToc({ title, items }: { title: string; items: readonly PageTocItem[] }) {
  const ref = useRef<HTMLElement>(null);
  const [active, setActive] = useState<string | null>(items[0]?.id ?? null);
  // A string, so the effect runs again only when the sections change, not on every render
  const ids = items.map((i) => i.id).join(" ");

  useEffect(() => {
    const scroller = scrollParent(ref.current);
    if (!scroller) return;
    const update = () => {
      const base = scroller.getBoundingClientRect().top;
      const tops = ids.split(" ").flatMap((id) => {
        const el = document.getElementById(id);
        return el ? [{ id, top: el.getBoundingClientRect().top - base }] : [];
      });
      const atBottom = scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - 2;
      setActive(activeSection(tops, READ_LINE, atBottom));
    };
    update();
    scroller.addEventListener("scroll", update, { passive: true });
    return () => scroller.removeEventListener("scroll", update);
  }, [ids]);

  const jump = (id: string) => {
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    document
      .getElementById(id)
      ?.scrollIntoView({ behavior: reduce ? "auto" : "smooth", block: "start" });
  };

  return (
    <nav ref={ref} className={styles.toc} aria-label={title}>
      <p className={styles.title}>{title}</p>
      {items.map(({ id, label, icon: Icon }) => (
        <button
          key={id}
          type="button"
          className={styles.item}
          aria-current={active === id ? "location" : undefined}
          onClick={() => jump(id)}
        >
          <Icon size={14} strokeWidth={1.85} aria-hidden="true" />
          <span>{label}</span>
        </button>
      ))}
    </nav>
  );
}

/** Returns the nearest ancestor that scrolls vertically. */
function scrollParent(el: HTMLElement | null): HTMLElement | null {
  for (let p = el?.parentElement ?? null; p; p = p.parentElement) {
    const { overflowY } = getComputedStyle(p);
    if (overflowY === "auto" || overflowY === "scroll") return p;
  }
  return null;
}
