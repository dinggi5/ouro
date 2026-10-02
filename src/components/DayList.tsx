// 하루 목록 — 행 44px, 선 없음(여백이 구분선). 왼쪽은 시각, 오른쪽은 제목. 일반 일정은 무채색이다(DESIGN «액센트 = AI»).
// 부탁은 같은 줄에 서되 점(◯ 대기 · ◔ 도는 중 · ● 답)과 액센트 색을 갖는다.

import { addDays, fmt, startOfDay } from "../lib/time";
import type { OuroEvent } from "../lib/events";
import { stateLabel, stateOf, unread, type Errand, type ErrandState, type Row } from "../lib/errands";

function timeLabel(e: OuroEvent, day: Date): string {
  if (e.allDay) return "종일";
  const start = e.startAt ?? 0;
  // 전날부터 이어진 일정은 시작 시각 대신 «이어서» — 그날 목록에서 00:00 처럼 보이면 거짓이다.
  return start < startOfDay(day).getTime() ? "이어서" : fmt.time.format(start);
}

function sub(e: OuroEvent, day: Date): string | null {
  if (e.allDay || e.startAt === null || e.endAt === null || e.endAt === e.startAt) return null;
  const nextDay = addDays(startOfDay(day), 1).getTime();
  return e.endAt > nextDay ? "다음 날까지" : `~ ${fmt.time.format(e.endAt)}`;
}

/** 부탁의 상태 점. 12px 한 장의 SVG — 색은 글자색(currentColor)을 따른다. */
export function ErrandDot({ state, fresh }: { state: ErrandState; fresh: boolean }) {
  const common = { width: 12, height: 12, viewBox: "0 0 12 12", "aria-hidden": true, className: "shrink-0" } as const;
  switch (state) {
    case "waiting":
      return (
        <svg {...common} className="shrink-0 text-accent">
          <circle cx="6" cy="6" r="4.5" fill="none" stroke="currentColor" strokeWidth="1.5" />
        </svg>
      );
    case "running":
      return (
        <svg {...common} className="shrink-0 text-accent">
          <circle cx="6" cy="6" r="4.5" fill="none" stroke="currentColor" strokeWidth="1.5" opacity="0.35" />
          <path d="M6 1.5A4.5 4.5 0 0 1 10.5 6" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
        </svg>
      );
    case "done":
      return (
        <svg {...common} className={`shrink-0 ${fresh ? "text-accent" : "text-ink-muted"}`}>
          <circle cx="6" cy="6" r="5" fill="currentColor" />
        </svg>
      );
    case "failed":
      return (
        <svg {...common} className="shrink-0 text-danger">
          <circle cx="6" cy="6" r="4.5" fill="none" stroke="currentColor" strokeWidth="1.5" />
          <path d="M6 3.5v3" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
        </svg>
      );
    default:
      return (
        <svg {...common} className="shrink-0 text-ink-muted">
          <circle cx="6" cy="6" r="4.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeDasharray="2.5 2" />
        </svg>
      );
  }
}

const rowClass =
  "-mx-3 flex min-h-11 w-[calc(100%+24px)] items-center gap-3 rounded-md px-3 py-2 text-left transition-colors duration-100 hover:bg-surface-sunken active:bg-hairline";

export function DayList({
  day,
  rows,
  now,
  onOpen,
  onOpenErrand,
}: {
  day: Date;
  rows: Row[];
  now: number;
  onOpen: (e: OuroEvent) => void;
  onOpenErrand: (e: Errand) => void;
}) {
  return (
    <ul className="flex flex-col">
      {rows.map((row) => {
        if (row.kind === "errand") {
          const r = row.r;
          const st = stateOf(r);
          return (
            <li key={`errand-${r.id}`}>
              <button type="button" onClick={() => onOpenErrand(r)} className={`${rowClass} text-ink`}>
                <span className="num w-16 shrink-0 text-caption text-ink-muted">{fmt.time.format(r.startAt)}</span>
                <ErrandDot state={st} fresh={unread(r)} />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-body-sm">{r.title}</span>
                  <span className={`block text-micro ${unread(r) ? "text-accent" : "text-ink-muted"}`}>
                    Claude Code · {stateLabel(r)}
                  </span>
                </span>
              </button>
            </li>
          );
        }
        const e = row.e;
        const past = !e.allDay && Math.max(e.endAt ?? 0, (e.startAt ?? 0) + 1) <= now;
        const end = sub(e, day);
        return (
          <li key={e.id}>
            <button type="button" onClick={() => onOpen(e)} className={`${rowClass} ${past ? "text-ink-muted" : "text-ink"}`}>
              <span className="num w-16 shrink-0 text-caption text-ink-muted">{timeLabel(e, day)}</span>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-body-sm">{e.title}</span>
                {end && <span className="num block text-micro text-ink-muted">{end}</span>}
              </span>
            </button>
          </li>
        );
      })}
    </ul>
  );
}
