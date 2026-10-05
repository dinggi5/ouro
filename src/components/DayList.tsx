// 하루 목록 — 행 44px, 선 없음(여백이 구분선). 왼쪽은 시각, 오른쪽은 제목. 일반 일정은 무채색이다(DESIGN «액센트 = AI»).
// 부탁은 같은 줄에 서되 엔소 점(열림 대기 · 앞만 짙음 도는 중 · 닫힘 답)과 액센트 색을 갖는다.

import { addDays, fmt, startOfDay } from "../lib/time";
import type { OuroEvent } from "../lib/events";
import { stateLabel, stateOf, targetLabel, unread, type Errand, type Row } from "../lib/errands";
import { ErrandDot } from "./Enso";

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
                    {targetLabel(r.target)}
                    {r.series && " · 반복"} · {stateLabel(r)}
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
