// 빠른 입력 — 러스트 `parse.rs` 가 돌려주는 초안과 그 문. 날짜 계산은 전부 러스트가 한다(CLAUDE.md «결정적 파서»);
// 여기선 카드에 보일 문장을 만들고 시트의 Draft 로 옮기기만 한다.

import { invoke } from "@tauri-apps/api/core";
import type { Draft } from "../components/EventSheet";
import { combine, fmt, parseYmd, sameDay, startOfDay } from "./time";

export type Miss = "no_date" | "invalid" | "repeat";

export type QuickDraft = {
  title: string;
  allDay: boolean;
  /** 날짜를 못 찾았으면 넷 다 null. */
  startDate: string | null;
  startTime: string | null;
  /** 종일이면 포함(화면 값). */
  endDate: string | null;
  endTime: string | null;
  warnings: string[];
  miss: Miss | null;
};

export const quickApi = {
  parse: (text: string, base: string | null) => invoke<QuickDraft>("parse_quick", { text, base }),
  recordMiss: (kind: Miss) => invoke<void>("record_parse_miss", { kind }),
};

/** 초안 → 시트 값. 날짜를 못 찾았으면 `fallback`(고른 날의 빈 시트)에 제목만 얹는다 — 사람이 날짜를 고른다. */
export function quickToDraft(q: QuickDraft, fallback: Draft): Draft {
  if (!q.startDate || !q.endDate) return { ...fallback, title: q.title };
  return {
    title: q.title,
    notes: "",
    allDay: q.allDay,
    startDate: q.startDate,
    startTime: q.startTime ?? "09:00",
    endDate: q.endDate,
    endTime: q.endTime ?? "10:00",
    alertMin: null,
  };
}

const REL = new Map([
  [-1, "어제"],
  [0, "오늘"],
  [1, "내일"],
  [2, "모레"],
]);

function dayLabel(d: Date, today: Date): string {
  const days = Math.round((startOfDay(d).getTime() - today.getTime()) / 86_400_000);
  const year = d.getFullYear() === today.getFullYear() ? "" : `${d.getFullYear()}년 `;
  const base = `${year}${fmt.monthDay.format(d)} ${fmt.weekday.format(d)}`;
  const rel = REL.get(days);
  return rel ? `${rel} · ${base}` : base;
}

/** 카드 둘째 줄: «내일 · 9월 30일 수요일 · 오후 3:00 – 오후 4:00». 날짜가 없으면 null. */
export function whenLabel(q: QuickDraft, today: Date): string | null {
  const s = q.startDate ? parseYmd(q.startDate) : null;
  const e = q.endDate ? parseYmd(q.endDate) : null;
  if (!s || !e) return null;
  if (q.allDay) {
    if (sameDay(s, e)) return `${dayLabel(s, today)} · 종일`;
    const days = Math.round((e.getTime() - s.getTime()) / 86_400_000) + 1;
    return `${dayLabel(s, today)} – ${fmt.monthDay.format(e)} · ${days}일`;
  }
  const st = combine(q.startDate ?? "", q.startTime ?? "");
  const et = combine(q.endDate ?? "", q.endTime ?? "");
  if (!st || !et) return dayLabel(s, today);
  const end = sameDay(st, et) ? fmt.time.format(et) : `${fmt.monthDay.format(et)} ${fmt.time.format(et)}`;
  return `${dayLabel(s, today)} · ${fmt.time.format(st)} – ${end}`;
}

