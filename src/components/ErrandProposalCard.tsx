// AI 의 부탁 제안 카드 — 목록 위에 뜬다. **보낼 원문이 그대로** 보이고, «승인» 해야만 부탁이 된다(그제야 때가 되면 돈다).
//
// 일정 제안 카드(`ProposalCard`)와 같은 틀 — 머리의 «엔소 … 부탁 제안» 과 «승인» 도장만 주홍.
// 글은 글자로만 그린다(`whitespace-pre-wrap`) — 바깥에서 온 글은 마크업도 명령도 아니다. 길면 상자 안에서 스크롤(카드가 목록을 덮지 않게).

import { LATE_OPTIONS, targetLabel, TOOL_OPTIONS } from "../lib/errands";
import type { ErrandProposal } from "../lib/proposals";
import { clientLabel, errandWhen } from "../lib/proposals";
import { Enso, SEAL } from "./Enso";

export function ErrandProposalCard({
  proposal: p,
  total,
  today,
  nowMs,
  busy,
  onApprove,
  onEdit,
  onReject,
}: {
  proposal: ErrandProposal;
  total: number;
  today: Date;
  nowMs: number;
  busy: boolean;
  onApprove: () => void;
  onEdit: () => void;
  onReject: () => void;
}) {
  const past = p.startAt < nowMs - 60_000;
  const meta = [
    `도구 ${TOOL_OPTIONS.find((o) => o.value === p.allowedTools)?.label ?? p.allowedTools}`,
    `놓치면 ${LATE_OPTIONS.find((o) => o.value === p.late)?.label ?? p.late}`,
    p.depth > 0 ? `답에서 이어진 부탁 ${p.depth}/3` : null,
  ].filter(Boolean);
  return (
    <section
      aria-label="AI 부탁 제안"
      className="toast-in mx-5 mt-4 flex shrink-0 flex-col gap-1 rounded-lg bg-surface px-4 pt-3 pb-4 ring-1 ring-hairline"
    >
      <div className="flex items-center justify-between gap-3 text-caption">
        <span className="flex min-w-0 items-center gap-1.5 text-accent">
          <Enso form="open" />
          <span className="truncate">
            {clientLabel(p.client)} 부탁 제안 · {targetLabel(p.target)} 에게
          </span>
        </span>
        {total > 1 && <span className="num shrink-0 text-ink-muted">1 / {total}</span>}
      </div>
      <p className={`num mt-1 text-body-sm font-semibold text-ink`}>
        {errandWhen(p.startAt, today)}
        {past && <span className="ml-2 text-caption font-normal">때가 지났어요 — 고쳐서 승인</span>}
      </p>
      <p className="select-text mt-1 max-h-28 overflow-y-auto whitespace-pre-wrap break-words rounded-md bg-surface-sunken px-3 py-2.5 text-caption text-ink">
        {p.prompt}
      </p>
      <p className="text-micro text-ink-muted">승인하면 때가 될 때 위 글이 그대로 {targetLabel(p.target)} 로 나가요 · {meta.join(" · ")}</p>
      <div className="mt-3 flex gap-2">
        <button
          type="button"
          disabled={busy || past}
          onClick={onApprove}
          className={SEAL}
        >
          승인
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={onEdit}
          className="h-10 rounded-md bg-surface-sunken px-4 text-label text-ink transition-colors duration-100 active:bg-hairline disabled:opacity-40"
        >
          고치기
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={onReject}
          className="h-10 rounded-md px-3 text-label text-ink-muted transition-colors duration-100 hover:text-ink-secondary disabled:opacity-40"
        >
          거절
        </button>
      </div>
    </section>
  );
}
