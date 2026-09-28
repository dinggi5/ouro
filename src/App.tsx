// 앱 루트 — 오늘/주/월 세 보기 + 일정 시트 + 되돌리기 토스트.
//
// 상태는 둘뿐이다: 보기(view) 와 커서(cursor, 고른 날). 세 보기가 같은 커서를 공유해서, 월에서 고른 날로
// «오늘» 탭을 누르면 그날 목록이 뜬다. 일정은 보기마다 필요한 창만 러스트에서 불러온다 — 캐시를 두지 않는다
// (쓰는 곳이 앱 하나라 목록이 틀릴 일이 없고, DB 가 로컬이라 한 번 읽는 데 1ms 도 안 걸린다).

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";
import { DayList } from "./components/DayList";
import { MonthGrid, monthGridRange } from "./components/MonthGrid";
import { blankDraft, EventSheet, fromEvent, type Draft } from "./components/EventSheet";
import { api, errorText, eventsOn, type EventInput, type OuroEvent } from "./lib/events";
import {
  addDays,
  addMonths,
  fmt,
  msUntilMidnight,
  parseYmd,
  sameDay,
  startOfDay,
  startOfWeek,
} from "./lib/time";

type View = "day" | "week" | "month";
const VIEWS: { id: View; label: string }[] = [
  { id: "day", label: "하루" },
  { id: "week", label: "주" },
  { id: "month", label: "월" },
];

/** 지금. 자정에 넘어가고, 창이 다시 보일 때도 다시 읽는다 — 맥이 잠든 사이 타이머는 늦게 깨므로 타이머 하나만 믿지 않는다.
 *  1분마다도 갱신한다 — 지난 일정을 흐리게 하는 기준이라서. */
function useNow(): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    let midnight: number;
    const arm = () => {
      midnight = window.setTimeout(() => {
        setNow(new Date());
        arm();
      }, msUntilMidnight(new Date()) + 50);
    };
    arm();
    const minute = window.setInterval(() => setNow(new Date()), 60_000);
    const onVisible = () => {
      if (document.visibilityState === "visible") setNow(new Date());
    };
    document.addEventListener("visibilitychange", onVisible);
    window.addEventListener("focus", onVisible);
    return () => {
      window.clearTimeout(midnight);
      window.clearInterval(minute);
      document.removeEventListener("visibilitychange", onVisible);
      window.removeEventListener("focus", onVisible);
    };
  }, []);
  return now;
}

function rangeOf(view: View, cursor: Date): [Date, Date] {
  if (view === "day") return [startOfDay(cursor), addDays(startOfDay(cursor), 1)];
  if (view === "week") {
    const s = startOfWeek(cursor);
    return [s, addDays(s, 7)];
  }
  return monthGridRange(cursor);
}

function title(view: View, cursor: Date): { main: string; sub: string } {
  if (view === "day") return { main: fmt.monthDay.format(cursor), sub: fmt.weekday.format(cursor) };
  if (view === "week") {
    const s = startOfWeek(cursor);
    const e = addDays(s, 6);
    return { main: `${fmt.monthDay.format(s)} – ${fmt.monthDay.format(e)}`, sub: `${s.getFullYear()}년` };
  }
  return { main: fmt.yearMonth.format(cursor), sub: `${fmt.monthDay.format(cursor)} ${fmt.weekday.format(cursor)}` };
}

type Sheet = { draft: Draft; editing: OuroEvent | null; key: number };
type Toast = { text: string; undo?: () => void; key: number };

function App() {
  const now = useNow();
  const today = startOfDay(now);
  const [view, setView] = useState<View>("day");
  const [cursor, setCursor] = useState<Date>(today);
  const [events, setEvents] = useState<OuroEvent[]>([]);
  const [fatal, setFatal] = useState<string | null>(null);
  const [sheet, setSheet] = useState<Sheet | null>(null);
  const [toast, setToast] = useState<Toast | null>(null);
  const sheetKey = useRef(0);

  // 자정이 지나면 «오늘» 에 머물던 커서도 따라 넘어간다. 다른 날을 보고 있었다면 그대로 둔다.
  const lastToday = useRef(today);
  useEffect(() => {
    if (!sameDay(lastToday.current, today)) {
      setCursor((c) => (sameDay(c, lastToday.current) ? today : c));
      lastToday.current = today;
    }
  }, [today]);

  const [from, to] = useMemo(() => rangeOf(view, cursor), [view, cursor]);
  const reload = useCallback(async () => {
    try {
      setEvents(await api.list(from, to));
      setFatal(null);
    } catch (e) {
      setFatal(errorText(e));
    }
  }, [from, to]);

  useEffect(() => {
    void reload();
    // 다시 보일 때 새로 읽는다 — 개발 4 부터는 MCP 로도 일정이 바뀐다.
    const onFocus = () => void reload();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [reload]);

  const showToast = useCallback((text: string, undo?: () => void) => {
    setToast({ text, undo, key: Date.now() });
  }, []);
  useEffect(() => {
    if (!toast) return;
    const t = window.setTimeout(() => setToast(null), 4000);
    return () => window.clearTimeout(t);
  }, [toast]);

  const openNew = useCallback(() => {
    setSheet({ draft: blankDraft(cursor, new Date()), editing: null, key: ++sheetKey.current });
  }, [cursor]);
  const openEdit = (e: OuroEvent) => setSheet({ draft: fromEvent(e), editing: e, key: ++sheetKey.current });

  const save = async (input: EventInput): Promise<string | null> => {
    try {
      const saved = sheet?.editing ? await api.update(sheet.editing.id, input) : await api.create(input);
      setSheet(null);
      // 만든 날로 커서를 옮긴다 — 다른 날에 만들었는데 목록에 안 보이면 «저장이 안 됐나» 한다.
      const day = saved.allDay ? parseYmd(saved.startDate ?? "") : new Date(saved.startAt ?? 0);
      if (!sheet?.editing && day && !sameDay(day, cursor)) setCursor(startOfDay(day));
      await reload();
      return null;
    } catch (e) {
      return errorText(e);
    }
  };

  const remove = async () => {
    const e = sheet?.editing;
    if (!e) return;
    try {
      await api.remove(e.id);
      setSheet(null);
      await reload();
      showToast("삭제했어요", async () => {
        setToast(null);
        try {
          await api.restore(e.id);
          await reload();
        } catch (err) {
          showToast(errorText(err));
        }
      });
    } catch (err) {
      showToast(errorText(err));
    }
  };

  const step = useCallback(
    (dir: -1 | 1) => {
      setCursor((c) => (view === "day" ? addDays(c, dir) : view === "week" ? addDays(c, 7 * dir) : addMonths(c, dir)));
    },
    [view],
  );

  // 키보드: ⌘W 닫기(창이 무테라 AppKit 이 안 준다, Kura 개발 58), ⌘N 새 일정, Esc 시트 닫기, ←/→ 넘기기, T 오늘.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const typing = e.target instanceof HTMLElement && e.target.closest("input, textarea, select");
      if (e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) {
        const k = e.key.toLowerCase();
        if (k === "w") {
          e.preventDefault();
          void invoke("hide_popover");
        } else if (k === "n" && !sheet) {
          e.preventDefault();
          openNew();
        }
        return;
      }
      if (e.key === "Escape" && sheet) {
        e.preventDefault();
        setSheet(null);
        return;
      }
      if (sheet || typing || e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key === "ArrowLeft") step(-1);
      else if (e.key === "ArrowRight") step(1);
      else if (e.key === "t" || e.key === "T") setCursor(today);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [sheet, openNew, step, today]);

  const t = title(view, cursor);
  const atToday = sameDay(cursor, today);
  const nowMs = now.getTime();

  return (
    <main className="relative flex h-screen w-full flex-col overflow-hidden rounded-lg bg-canvas text-ink">
      <header className="shrink-0 px-5 pt-6">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <h1 className="num truncate text-subheading font-semibold">{t.main}</h1>
            <p className="num mt-1 text-body-sm text-ink-muted">{t.sub}</p>
          </div>
          <div className="flex shrink-0 items-center gap-1 pt-0.5">
            {!atToday && (
              <button
                type="button"
                onClick={() => setCursor(today)}
                className="h-8 rounded-pill px-3 text-label text-ink-secondary transition-colors duration-100 hover:bg-surface-sunken active:bg-hairline"
              >
                오늘
              </button>
            )}
            <IconButton label="이전" onClick={() => step(-1)}>
              <path d="M10 3.5 5.5 8l4.5 4.5" />
            </IconButton>
            <IconButton label="다음" onClick={() => step(1)}>
              <path d="M6 3.5 10.5 8 6 12.5" />
            </IconButton>
            <IconButton label="새 일정" onClick={openNew}>
              <path d="M8 3v10M3 8h10" />
            </IconButton>
          </div>
        </div>

        <div role="tablist" className="mt-5 grid grid-cols-3 rounded-md bg-surface-sunken p-1">
          {VIEWS.map((v) => (
            <button
              key={v.id}
              type="button"
              role="tab"
              aria-selected={view === v.id}
              onClick={() => setView(v.id)}
              className={`h-8 rounded-sm text-label transition-colors duration-100 ${
                view === v.id ? "bg-surface font-semibold text-ink dark:bg-hairline" : "text-ink-muted hover:text-ink-secondary"
              }`}
            >
              {v.label}
            </button>
          ))}
        </div>
      </header>

      <section className="min-h-0 flex-1 overflow-y-auto px-5 pt-4 pb-5">
        {fatal ? (
          <Empty text={fatal} />
        ) : view === "day" ? (
          <DayBody day={cursor} events={events} now={nowMs} onOpen={openEdit} onNew={openNew} />
        ) : view === "week" ? (
          <WeekBody
            cursor={cursor}
            today={today}
            events={events}
            now={nowMs}
            onOpen={openEdit}
            onPickDay={(d) => {
              setCursor(d);
              setView("day");
            }}
          />
        ) : (
          <>
            <MonthGrid cursor={cursor} today={today} events={events} onPick={setCursor} />
            <div className="mt-4">
              <DayBody day={cursor} events={events} now={nowMs} onOpen={openEdit} onNew={openNew} />
            </div>
          </>
        )}
      </section>

      {sheet && (
        <>
          <div className="scrim-in absolute inset-0 bg-black/30" onClick={() => setSheet(null)} />
          <EventSheet
            key={sheet.key}
            initial={sheet.draft}
            editing={sheet.editing}
            onSave={save}
            onDelete={() => void remove()}
            onClose={() => setSheet(null)}
          />
        </>
      )}

      {toast && (
        <div
          key={toast.key}
          role="status"
          className="toast-in absolute inset-x-0 bottom-5 mx-auto flex w-fit items-center gap-4 rounded-pill bg-[rgba(25,31,40,0.92)] px-5 py-3 text-body-sm text-white"
        >
          <span>{toast.text}</span>
          {toast.undo && (
            <button type="button" onClick={toast.undo} className="font-semibold text-white/70 hover:text-white">
              되돌리기
            </button>
          )}
        </div>
      )}
    </main>
  );
}

function IconButton({ label, onClick, children }: { label: string; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className="flex size-8 items-center justify-center rounded-pill text-ink-secondary transition-colors duration-100 hover:bg-surface-sunken active:bg-hairline"
    >
      <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
        {children}
      </svg>
    </button>
  );
}

function Empty({ text, action }: { text: string; action?: { label: string; onClick: () => void } }) {
  return (
    <div className="flex flex-col items-center gap-3 py-12">
      <p className="text-body-sm text-ink-muted">{text}</p>
      {action && (
        <button
          type="button"
          onClick={action.onClick}
          className="h-9 rounded-pill bg-surface-sunken px-4 text-label text-ink-secondary transition-colors duration-100 active:bg-hairline"
        >
          {action.label}
        </button>
      )}
    </div>
  );
}

function DayBody(props: { day: Date; events: OuroEvent[]; now: number; onOpen: (e: OuroEvent) => void; onNew: () => void }) {
  const list = eventsOn(props.events, props.day);
  if (list.length === 0) return <Empty text="비어 있어요" action={{ label: "일정 만들기", onClick: props.onNew }} />;
  return <DayList day={props.day} events={list} now={props.now} onOpen={props.onOpen} />;
}

function WeekBody(props: {
  cursor: Date;
  today: Date;
  events: OuroEvent[];
  now: number;
  onOpen: (e: OuroEvent) => void;
  onPickDay: (d: Date) => void;
}) {
  const start = startOfWeek(props.cursor);
  const days = Array.from({ length: 7 }, (_, i) => addDays(start, i));
  return (
    <div className="flex flex-col gap-4">
      {days.map((d) => {
        const list = eventsOn(props.events, d);
        const isToday = sameDay(d, props.today);
        return (
          <div key={d.getTime()}>
            <button
              type="button"
              onClick={() => props.onPickDay(d)}
              className={`num flex h-7 items-center gap-2 text-caption ${
                isToday ? "font-semibold text-ink" : list.length ? "text-ink-secondary" : "text-ink-muted"
              }`}
            >
              <span>{fmt.monthDay.format(d)}</span>
              <span>{fmt.weekdayShort.format(d)}</span>
              {isToday && <span>· 오늘</span>}
            </button>
            {list.length > 0 && <DayList day={d} events={list} now={props.now} onOpen={props.onOpen} />}
          </div>
        );
      })}
    </div>
  );
}

export default App;
