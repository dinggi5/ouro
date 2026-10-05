// 엔소(円相, 붓 한 번 원) — 우로의 표지(개발 13). 모양은 `scripts/enso.mjs` 가 만든 `lib/enso.ts` 하나.
// 열린 원 = 아직 끝나지 않은 부탁, 앞부분만 짙은 원 = 도는 중, 닫혀 채워진 원 = 답이 왔다.
// 색은 글자색(currentColor)을 따른다 — 부르는 쪽이 text-accent(AI) 나 text-ink-muted 를 정한다.

import { useEffect, useId, useState } from "react";
import { ENSO_PATH, ENSO_R, ENSO_RUN_CLIP } from "../lib/enso";
import type { ErrandState } from "../lib/errands";

export type EnsoForm = "open" | "running" | "closed";

// 의식 모션(DESIGN «모션», 800ms): 틈을 메우는 호가 그어지고(0–500) 안이 채워진다(500–800). 틈은 268°→328°(enso.mjs A2·A1).
const GAP_ARC = (() => {
  const r = ENSO_R - 1.2;
  const p = (d: number) => [12 + r * Math.cos((d * Math.PI) / 180), 12 + r * Math.sin((d * Math.PI) / 180)].map((n) => n.toFixed(2)).join(" ");
  return `M${p(244)}A${r} ${r} 0 0 1 ${p(334)}`;
})();

export function Enso({ form, size = 12, className = "", ritual = false }: { form: EnsoForm; size?: number; className?: string; ritual?: boolean }) {
  const clip = useId();
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true" className={`shrink-0 ${className}`} fill="currentColor">
      {form === "open" && <path d={ENSO_PATH} />}
      {form === "running" && (
        <>
          <defs>
            <clipPath id={clip}>
              <path d={ENSO_RUN_CLIP} />
            </clipPath>
          </defs>
          <path d={ENSO_PATH} opacity="0.35" />
          <path d={ENSO_PATH} clipPath={`url(#${clip})`} />
        </>
      )}
      {form === "closed" &&
        (ritual ? (
          <>
            <path d={ENSO_PATH} />
            <path className="enso-stroke" d={GAP_ARC} pathLength={1} fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" />
            <circle className="enso-fill" cx="12" cy="12" r={ENSO_R} />
          </>
        ) : (
          <circle cx="12" cy="12" r={ENSO_R} />
        ))}
    </svg>
  );
}

/** 부탁의 상태 점(12px). 실패·건너뜀·멈춤은 색 없이 열린 원 — 끝나지 못한 고리다(실패만 한 단 짙게). 글(«실패» 등)은 옆 줄이 말한다. */
export function ErrandDot({ state, fresh }: { state: ErrandState; fresh: boolean }) {
  // 보고 있는 사이 답이 도착하면(도는 중·대기 → 답) 한 번만 고리를 닫는다. 처음 그릴 때 이미 답이면 그냥 닫힌 원.
  // 렌더 중에 앞 상태와 견준다(React «props 로 상태 고치기» 꼴) — effect 로 하면 닫힌 원이 한 프레임 먼저 보인다.
  const [last, setLast] = useState(state);
  const [ritual, setRitual] = useState(false);
  if (state !== last) {
    setLast(state);
    if (state === "done" && (last === "running" || last === "waiting")) setRitual(true);
  }
  useEffect(() => {
    if (!ritual) return;
    const t = setTimeout(() => setRitual(false), 800);
    return () => clearTimeout(t);
  }, [ritual]);

  switch (state) {
    case "waiting":
      return <Enso form="open" className="text-accent" />;
    case "running":
      return <Enso form="running" className="text-accent" />;
    case "done":
      return <Enso form="closed" ritual={ritual} className={fresh ? "text-accent" : "text-ink-muted"} />;
    case "failed":
      return <Enso form="open" className="text-ink-secondary" />;
    default:
      return <Enso form="open" className="text-ink-muted" />;
  }
}

/** AI 제안을 받는 단추(«넣기»·«승인») — 인장(朱肉)처럼 주홍 글자에 주홍 테. 사람의 승인 = 도장(개발 13).
 *  다른 주 버튼(저장)은 먹색 그대로 — 주홍은 «AI 가 관여한 것» 에만. */
export const SEAL =
  "h-10 flex-1 rounded-md text-label font-semibold text-accent shadow-[inset_0_0_0_1.5px_var(--accent)] transition-colors duration-100 hover:bg-surface-sunken active:bg-hairline disabled:opacity-40";
