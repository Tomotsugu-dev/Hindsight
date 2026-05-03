import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import styles from "./Section.module.css";

interface SectionProps {
  title: string;
  description?: string;
  icon?: LucideIcon;
  tone?: "primary" | "danger";
  children: ReactNode;
}

export function Section({
  title,
  description,
  icon: Icon,
  tone = "primary",
  children,
}: SectionProps) {
  return (
    <section className={styles.section}>
      <header className={styles.header}>
        {Icon ? (
          <div className={`${styles.icon} ${styles[`tone_${tone}`]}`}>
            <Icon size={20} strokeWidth={1.85} />
          </div>
        ) : null}
        <div className={styles.headText}>
          <h2 className={styles.title}>{title}</h2>
          {description ? (
            <p className={styles.description}>{description}</p>
          ) : null}
        </div>
      </header>
      <div className={styles.card}>{children}</div>
    </section>
  );
}
