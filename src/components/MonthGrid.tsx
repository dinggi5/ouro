// 작은 월 달력 — 6주 격자(달마다 높이가 흔들리지 않게 늘 6줄). 일정이 있는 날엔 점 하나.
// 고른 날 = 먹색 원, 오늘 = 옅은 원. 색은 쓰지 않는다(액센트는 AI 몫).

import { addDays, sameDay, startOfMonth, startOfWeek } from "../lib/time";
import type { OuroEvent } from "../lib/events";
import { onDay } from "../lib/events";
import type { Errand } from "../lib/errands";

const WEEKDAYS = ["일", "월", "화", "수", "목", "금", "토"];

/** 이 달 격자의 첫 칸(그 주 일요일)과 끝(배타). App 이 이 창으로 일정을 불러온다. */
export function monthGridRange(cursor: Date): [Date, Date] {
  const first = startOfWeek(startOfMonth(cursor));
  return [first, addDays(first, 42)];
}

export function MonthGrid({
  cursor,
  today,
  events,
  errands,
  onPick,
}: {
  cursor: Date;
  today: Date;
  events: OuroEvent[];
  errands: Errand[];
  onPick: (d: Date) => void;
}) {
  const [first] = monthGridRange(cursor);
  const days = Array.from({ length: 42 }, (_, i) => addDays(first, i));

  return (
    <div role="grid" aria-label="월 달력">
      <div className="grid grid-cols-7 pb-1">
        {WEEKDAYS.map((w) => (
          <span key={w} className="text-center text-micro text-ink-muted">
            {w}
          </span>
        ))}
      </div>
      <div className="grid grid-cols-7">
        {days.map((d) => {
          const inMonth = d.getMonth() === cursor.getMonth();
          const selected = sameDay(d, cursor);
          const isToday = sameDay(d, today);
          const dayMs = d.getTime();
          const has = events.some((e) => onDay(e, d)) || errands.some((r) => r.startAt >= dayMs && r.startAt < addDays(d, 1).getTime());
          return (
            <button
              key={d.getTime()}
              type="button"
              role="gridcell"
              aria-selected={selected}
              onClick={() => onPick(d)}
              className="flex h-10 flex-col items-center justify-center gap-0.5"
            >
              <span
                className={`num flex size-7 items-center justify-center rounded-full text-label transition-colors duration-100 ${
                  selected
                    ? "bg-ink font-semibold text-canvas"
                    : isToday
                      ? "bg-surface-sunken font-semibold text-ink"
                      : inMonth
                        ? "text-ink hover:bg-surface-sunken"
                        : "text-ink-muted hover:bg-surface-sunken"
                }`}
              >
                {d.getDate()}
              </span>
              <span className={`size-1 rounded-full ${has ? "bg-ink-muted" : "bg-transparent"}`} />
            </button>
          );
        })}
      </div>
    </div>
  );
}
