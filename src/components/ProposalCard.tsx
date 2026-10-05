// AI 의 일정 제안 카드 — 목록 위에 뜬다. 한 번 눌러 넣고(«넣기»), 고쳐서 넣고, 거절한다.
//
// 색: 카드 머리의 «엔소 Claude Code 가 제안» 이 액센트다(DESIGN «액센트 = AI 가 관여한 것»). «넣기» 는 주홍 도장(`SEAL`) — 사람의 승인이 찍히는 자리다(개발 13).
// 여러 개면 먼저 온 것 하나만 보이고 «1 / 3» — 카드가 쌓여 목록을 덮지 않게.

import type { Proposal } from "../lib/proposals";
import { clientLabel, proposalWhen } from "../lib/proposals";
import { Enso, SEAL } from "./Enso";

export function ProposalCard({
  proposal: p,
  total,
  today,
  busy,
  onApprove,
  onEdit,
  onReject,
}: {
  proposal: Proposal;
  total: number;
  today: Date;
  busy: boolean;
  onApprove: () => void;
  onEdit: () => void;
  onReject: () => void;
}) {
  return (
    <section
      aria-label="AI 일정 제안"
      className="toast-in mx-5 mt-4 flex shrink-0 flex-col gap-1 rounded-lg bg-surface px-4 pt-3 pb-4 ring-1 ring-hairline"
    >
      <div className="flex items-center justify-between gap-3 text-caption">
        <span className="flex min-w-0 items-center gap-1.5 text-accent">
          <Enso form="open" />
          <span className="truncate">{clientLabel(p.client)} 제안</span>
        </span>
        {total > 1 && <span className="num shrink-0 text-ink-muted">1 / {total}</span>}
      </div>
      <p className="mt-1 truncate text-body-sm font-semibold text-ink">{p.event.title}</p>
      <p className="num text-caption text-ink-secondary">{proposalWhen(p.event, today)}</p>
      {p.conflicts.length > 0 && (
        <p className="truncate text-caption text-ink-muted">겹침 · {p.conflicts.join(", ")}</p>
      )}
      {p.event.notes && <p className="line-clamp-2 text-caption text-ink-muted">{p.event.notes}</p>}
      <div className="mt-3 flex gap-2">
        <button
          type="button"
          disabled={busy}
          onClick={onApprove}
          className={SEAL}
        >
          넣기
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
