// 일정 만들기·고치기 시트. 팝오버 아래에서 올라온다(320ms, 애플 레이아웃 속도).
//
// 날짜·시각 칸은 WebKit 기본 `<input type="date|time">` 을 쓴다 — 키보드로 칸마다 숫자를 넣을 수 있고, 시스템 로캘을 따른다.
// 자연어 한 줄 입력(「내일 3시 치과」)은 팝오버 아래 `QuickBar` — 날짜를 못 알아들었거나 ⌘↩ 면 그 초안으로 이 시트가 열린다.
//
// 종일 일정의 끝 날짜는 화면에선 **포함**(9/28 ~ 9/28 = 하루), 저장은 **배타**(end = 9/29). 바꾸는 곳은 toInput·fromEvent 두 곳뿐.

import { useEffect, useRef, useState } from "react";
import { addDays, combine, hm, nextHour, parseYmd, sameDay, ymd } from "../lib/time";
import { ALERTS_ALL_DAY, ALERTS_TIMED, type EventInput, type OuroEvent } from "../lib/events";

type Draft = {
  title: string;
  notes: string;
  allDay: boolean;
  startDate: string;
  startTime: string;
  endDate: string; // 화면 값 — 종일이면 포함
  endTime: string;
  alertMin: number | null;
  private: boolean;
  /** 고치는 일정의 원래 순간. 날짜·시각 칸을 안 건드렸으면 이 값을 그대로 쓴다 — 가을 서머타임의 두 번째 01:30 은
   *  «01:30» 글자만으론 첫 번째 01:30 으로 되돌아가, 제목만 고쳐도 일정이 한 시간 앞당겨진다(코덱스 개발 4). */
  orig?: { startKey: string; startAt: number; endKey: string; endAt: number };
};

const HOUR = 60 * 60 * 1000;

/** 새 일정의 기본값: 고른 날의 (오늘이면 다음 정각, 아니면 오전 9시)부터 한 시간. */
export function blankDraft(day: Date, now: Date): Draft {
  const start = sameDay(day, now) ? nextHour(now) : new Date(day.getFullYear(), day.getMonth(), day.getDate(), 9);
  const end = new Date(start.getTime() + HOUR);
  return {
    title: "",
    notes: "",
    allDay: false,
    startDate: ymd(start),
    startTime: hm(start),
    endDate: ymd(end),
    endTime: hm(end),
    alertMin: null,
    private: false,
  };
}

function fromEvent(e: OuroEvent): Draft {
  if (e.allDay) {
    const endIncl = addDays(parseYmd(e.endDate ?? "") ?? new Date(), -1);
    return {
      title: e.title,
      notes: e.notes,
      allDay: true,
      startDate: e.startDate ?? "",
      startTime: "09:00",
      endDate: ymd(endIncl),
      endTime: "10:00",
      alertMin: e.alertMin,
      private: e.private,
    };
  }
  const s = new Date(e.startAt ?? 0);
  const x = new Date(e.endAt ?? 0);
  const orig = { startKey: `${ymd(s)} ${hm(s)}`, startAt: s.getTime(), endKey: `${ymd(x)} ${hm(x)}`, endAt: x.getTime() };
  return {
    orig,
    title: e.title,
    notes: e.notes,
    allDay: false,
    startDate: ymd(s),
    startTime: hm(s),
    endDate: ymd(x),
    endTime: hm(x),
    alertMin: e.alertMin,
    private: e.private,
  };
}

/** 화면 값 → 저장 모양. 모양이 틀리면 사람에게 보일 한 줄. */
function toInput(d: Draft): EventInput | string {
  if (!d.title.trim()) return "제목을 적어 주세요";
  if (d.allDay) {
    const s = parseYmd(d.startDate);
    const e = parseYmd(d.endDate);
    if (!s || !e) return "날짜를 확인해 주세요";
    if (e < s) return "끝나는 날이 시작보다 앞이에요";
    return {
      title: d.title,
      notes: d.notes,
      allDay: true,
      startAt: null,
      endAt: null,
      startDate: ymd(s),
      endDate: ymd(addDays(e, 1)),
      alertMin: d.alertMin,
      private: d.private,
    };
  }
  const o = d.orig;
  const s = o && `${d.startDate} ${d.startTime}` === o.startKey ? new Date(o.startAt) : combine(d.startDate, d.startTime);
  const e = o && `${d.endDate} ${d.endTime}` === o.endKey ? new Date(o.endAt) : combine(d.endDate, d.endTime);
  // 모양이 맞는데 null 이면 서머타임으로 건너뛰는 시각이다(time.ts combine).
  if (!s || !e) return "없는 시각이에요 — 날짜와 시각을 확인해 주세요";
  if (e < s) return "끝나는 시각이 시작보다 앞이에요";
  return {
    title: d.title,
    notes: d.notes,
    allDay: false,
    startAt: s.getTime(),
    endAt: e.getTime(),
    startDate: null,
    endDate: null,
    alertMin: d.alertMin,
    private: d.private,
  };
}

/** 시작을 옮기면 끝도 같은 길이만큼 따라간다(캘린더 앱의 관용). 길이를 못 재면 끝은 그대로. */
function shiftStart(d: Draft, next: Partial<Pick<Draft, "startDate" | "startTime">>): Draft {
  const moved = { ...d, ...next };
  if (d.allDay) {
    const s0 = parseYmd(d.startDate);
    const e0 = parseYmd(d.endDate);
    const s1 = parseYmd(moved.startDate);
    if (!s0 || !e0 || !s1) return moved;
    const days = Math.round((e0.getTime() - s0.getTime()) / (24 * HOUR));
    return { ...moved, endDate: ymd(addDays(s1, Math.max(days, 0))) };
  }
  const s0 = combine(d.startDate, d.startTime);
  const e0 = combine(d.endDate, d.endTime);
  const s1 = combine(moved.startDate, moved.startTime);
  if (!s0 || !e0 || !s1) return moved;
  const e1 = new Date(s1.getTime() + Math.max(e0.getTime() - s0.getTime(), 0));
  return { ...moved, endDate: ymd(e1), endTime: hm(e1) };
}

const field =
  "h-11 rounded-md bg-surface-sunken px-3 text-body-sm text-ink outline-none transition-shadow duration-100 focus:shadow-[inset_0_0_0_1.5px_var(--ink-secondary)]";

export function EventSheet({
  initial,
  editing,
  notice,
  heading,
  onSave,
  onDelete,
  onClose,
  docked = false,
}: {
  initial: Draft;
  editing: OuroEvent | null;
  /** 빠른 입력이 남긴 경고 — 시트 위에 그대로 보인다(왜 바로 안 넣었는지). */
  notice?: string[];
  /** 머리 글자를 바꾼다(AI 제안을 고칠 때 «제안 고치기»). */
  heading?: string;
  onSave: (input: EventInput) => Promise<string | null>;
  onDelete: () => void;
  onClose: () => void;
  /** 크게 보기 창의 오른쪽 칸에 붙어 있다(개발 12) — 올라오는 모션·둥근 위 모서리 없이 칸을 꽉 채운다. */
  docked?: boolean;
}) {
  const [d, setD] = useState<Draft>(initial);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const titleRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    // 올라오는 모션이 끝난 뒤 포커스 — 도중에 주면 WebKit 이 스크롤로 끌어올려 모션이 튄다.
    const t = window.setTimeout(() => titleRef.current?.focus(), 320);
    return () => window.clearTimeout(t);
  }, []);

  const set = (patch: Partial<Draft>) => {
    setError(null);
    setD((cur) => ({ ...cur, ...patch }));
  };

  const submit = async (ev?: React.FormEvent) => {
    ev?.preventDefault();
    if (busy) return;
    const input = toInput(d);
    if (typeof input === "string") {
      setError(input);
      return;
    }
    setBusy(true);
    const err = await onSave(input);
    setBusy(false);
    if (err) setError(err);
  };

  const alerts = d.allDay ? ALERTS_ALL_DAY : ALERTS_TIMED;

  return (
    <form
      onSubmit={submit}
      onKeyDown={(e) => {
        // ⌘↩ = 메모 칸 안에서도 저장. 그냥 ↩ 는 메모에선 줄바꿈, 다른 칸에선 form 이 알아서 제출한다.
        if (e.key === "Enter" && e.metaKey) void submit();
      }}
      className={docked ? "absolute inset-0 flex flex-col bg-surface" : "sheet-in absolute inset-x-0 bottom-0 top-3 flex flex-col rounded-t-xl bg-surface"}
      aria-label={editing ? "일정 고치기" : "새 일정"}
    >
      <header className="flex h-12 shrink-0 items-center justify-between px-5">
        <button type="button" onClick={onClose} className="text-body-sm text-ink-secondary active:text-ink">
          취소
        </button>
        <span className="text-label font-semibold">{heading ?? (editing ? "일정" : "새 일정")}</span>
        <span className="w-8" />
      </header>

      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-5 pb-4">
        {notice?.map((n) => (
          <p key={n} className="text-caption text-ink-muted">
            {n}
          </p>
        ))}
        <input
          ref={titleRef}
          value={d.title}
          onChange={(e) => set({ title: e.target.value })}
          placeholder="제목"
          maxLength={200}
          className="h-12 bg-transparent text-body font-semibold text-ink outline-none placeholder:text-ink-muted"
        />

        <label className="flex h-11 items-center justify-between">
          <span className="text-body-sm">종일</span>
          <input
            type="checkbox"
            checked={d.allDay}
            onChange={(e) =>
              set({
                allDay: e.target.checked,
                // 종일 ↔ 시각을 바꾸면 알림 선택지가 달라진다 — 없는 값이 남지 않게 비운다.
                alertMin: null,
              })
            }
            className="switch"
          />
        </label>

        <div className="grid grid-cols-[2.5rem_minmax(0,1fr)_8.5rem] items-center gap-2">
          <span className="text-caption text-ink-muted">시작</span>
          <input
            type="date"
            value={d.startDate}
            onChange={(e) => {
              setError(null);
              setD((cur) => shiftStart(cur, { startDate: e.target.value }));
            }}
            className={`${field} num ${d.allDay ? "col-span-2" : ""}`}
          />
          {!d.allDay && (
            <input
              type="time"
              value={d.startTime}
              onChange={(e) => {
                setError(null);
                setD((cur) => shiftStart(cur, { startTime: e.target.value }));
              }}
              className={`${field} num`}
            />
          )}
          <span className="text-caption text-ink-muted">끝</span>
          <input
            type="date"
            value={d.endDate}
            onChange={(e) => set({ endDate: e.target.value })}
            className={`${field} num ${d.allDay ? "col-span-2" : ""}`}
          />
          {!d.allDay && (
            <input
              type="time"
              value={d.endTime}
              onChange={(e) => set({ endTime: e.target.value })}
              className={`${field} num`}
            />
          )}
        </div>

        <label className="grid grid-cols-[2.5rem_1fr] items-center gap-2">
          <span className="text-caption text-ink-muted">알림</span>
          <select
            value={d.alertMin === null ? "" : String(d.alertMin)}
            onChange={(e) => set({ alertMin: e.target.value === "" ? null : Number(e.target.value) })}
            className={field}
          >
            {alerts.map((a) => (
              <option key={a.label} value={a.min === null ? "" : String(a.min)}>
                {a.label}
              </option>
            ))}
          </select>
        </label>

        <label className="flex h-11 items-center justify-between">
          <span className="text-body-sm">AI 에게 숨기기</span>
          <input
            type="checkbox"
            checked={d.private}
            onChange={(e) => set({ private: e.target.checked })}
            className="switch"
          />
        </label>

        <textarea
          value={d.notes}
          onChange={(e) => set({ notes: e.target.value })}
          placeholder="메모"
          rows={3}
          className="min-h-20 resize-none rounded-md bg-surface-sunken px-3 py-2.5 text-body-sm text-ink outline-none transition-shadow duration-100 placeholder:text-ink-muted focus:shadow-[inset_0_0_0_1.5px_var(--ink-secondary)]"
        />
      </div>

      <footer className="flex shrink-0 flex-col gap-2 px-5 pb-5">
        {error && (
          <p role="alert" className="text-caption text-danger">
            {error}
          </p>
        )}
        <div className="flex gap-2">
          {editing && (
            <button
              type="button"
              onClick={onDelete}
              className="h-13 rounded-md bg-surface-sunken px-5 text-body-sm font-semibold text-danger transition-colors duration-100 active:bg-hairline"
            >
              삭제
            </button>
          )}
          <button
            type="submit"
            disabled={busy}
            className="h-13 flex-1 rounded-md bg-ink text-body-sm font-semibold text-canvas transition-opacity duration-100 active:opacity-80 disabled:opacity-40"
          >
            저장
          </button>
        </div>
      </footer>
    </form>
  );
}

export { fromEvent, toInput };
export type { Draft };
