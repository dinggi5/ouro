// 일정 — 러스트 `store.rs` 와 오가는 모양과 그 문(invoke). 검사는 러스트가 한다; 여기선 화면이 쓰기 좋게만 다듬는다.

import { invoke } from "@tauri-apps/api/core";
import { addDays, parseYmd, startOfDay } from "./time";

export type OuroEvent = {
  id: number;
  title: string;
  notes: string;
  allDay: boolean;
  /** 시각 일정: UTC ms. 종일이면 null. */
  startAt: number | null;
  endAt: number | null;
  /** 종일 일정: 로컬 `YYYY-MM-DD`, 끝은 배타. 시각 일정이면 null. */
  startDate: string | null;
  endDate: string | null;
  alertMin: number | null;
  updatedAt: number;
};

export type EventInput = Omit<OuroEvent, "id" | "updatedAt">;

/** 알림 선택지(분 전) — `store.rs` 의 ALERT_CHOICES 와 같아야 한다. 종일 일정은 오전 9시 기준. */
export const ALERTS_TIMED: { min: number | null; label: string }[] = [
  { min: null, label: "없음" },
  { min: 0, label: "시작할 때" },
  { min: 5, label: "5분 전" },
  { min: 10, label: "10분 전" },
  { min: 15, label: "15분 전" },
  { min: 30, label: "30분 전" },
  { min: 60, label: "1시간 전" },
  { min: 120, label: "2시간 전" },
  { min: 1440, label: "하루 전" },
];
export const ALERTS_ALL_DAY: { min: number | null; label: string }[] = [
  { min: null, label: "없음" },
  { min: 0, label: "당일 오전 9시" },
  { min: 1440, label: "전날 오전 9시" },
];

export const api = {
  list: (from: Date, to: Date) =>
    invoke<OuroEvent[]>("list_events", { from: from.getTime(), to: to.getTime() }),
  create: (input: EventInput) => invoke<OuroEvent>("create_event", { input }),
  update: (id: number, input: EventInput) => invoke<OuroEvent>("update_event", { id, input }),
  remove: (id: number) => invoke<void>("delete_event", { id }),
  restore: (id: number) => invoke<OuroEvent>("restore_event", { id }),
};

/** 이 일정이 로컬 날짜 `day` 에 걸치나. 러스트 list_events 의 겹침 규칙과 같다(길이 0 인 일정은 그 순간이 속한 날). */
export function onDay(e: OuroEvent, day: Date): boolean {
  const start = startOfDay(day);
  const end = addDays(start, 1);
  if (e.allDay) {
    const s = parseYmd(e.startDate ?? "");
    const x = parseYmd(e.endDate ?? "");
    return !!s && !!x && s < end && x > start;
  }
  const s = e.startAt ?? 0;
  const x = Math.max(e.endAt ?? s, s + 1);
  return s < end.getTime() && x > start.getTime();
}

/** 하루 목록의 순서: 종일이 위, 그다음 시작 시각. */
export function byDayOrder(a: OuroEvent, b: OuroEvent): number {
  if (a.allDay !== b.allDay) return a.allDay ? -1 : 1;
  if (a.allDay) return (a.startDate ?? "").localeCompare(b.startDate ?? "") || a.id - b.id;
  return (a.startAt ?? 0) - (b.startAt ?? 0) || a.id - b.id;
}

export function eventsOn(events: OuroEvent[], day: Date): OuroEvent[] {
  return events.filter((e) => onDay(e, day)).sort(byDayOrder);
}

/** invoke 가 던진 것 → 사람에게 보일 한 줄. 러스트 커맨드는 문장을 문자열로 던진다. */
export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : "알 수 없는 오류";
}
