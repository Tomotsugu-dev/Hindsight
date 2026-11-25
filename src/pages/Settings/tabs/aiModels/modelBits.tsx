import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { RecommendedModel } from "../../../../api/hindsight";
import { BRAND_LOGOS } from "../../../AISettings/shared/brandLogos";
import s from "./aiModels.module.css";

/** The brand's logo, or the first letter of the model's name when there is none or it fails to load. */
export function ModelLogo({ rec }: { rec: RecommendedModel }) {
  const src = BRAND_LOGOS[rec.brand] ?? rec.logoUrl;
  const [failed, setFailed] = useState(false);
  if (!src || failed) {
    return (
      <span className={s.tile} aria-hidden>
        {rec.displayName.charAt(0)}
      </span>
    );
  }
  return (
    // onError 是图片加载事件，不是用户操作
    // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions
    <img
      className={s.logo}
      src={src}
      alt=""
      loading="lazy"
      referrerPolicy="no-referrer"
      onError={() => setFailed(true)}
    />
  );
}

/** R1 models also have REASONING, and their names already say R1. */
const HIDDEN_CAPS = new Set(["R1"]);

/** What a recommended model can do, in words. A cap the app has no words for shows as it is. */
export function CapTags({ caps }: { caps: string[] }) {
  const { t } = useTranslation();
  return (
    <>
      {caps
        .filter((cap) => !HIDDEN_CAPS.has(cap))
        .map((cap) => (
          <span
            key={cap}
            className={s.cap}
            title={
              t(`aiSettings.models.card.capsTooltips.${cap}`, { defaultValue: "" }) || undefined
            }
          >
            {t(`settings.aiModels.caps.${cap}`, { defaultValue: cap })}
          </span>
        ))}
    </>
  );
}
