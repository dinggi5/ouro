// 부탁 — 러스트 `errands.rs` 와 오가는 모양과 그 문. 부탁 = «언제 · 누구에게 · 무엇을», 돈 기록 = 실행(run).
// 바깥으로 나가는 문장은 `prompt` 하나(사람이 쓴 것). 답(`run.response`)은 화면에 **글자로만** 그린다 — 어디서도 문장이 되지 않는다(CLAUDE.md).

import { invoke } from "@tauri-apps/api/core";
import { addDays, startOfDay } from "./time";
import { byDayOrder, eventsOn, type OuroEvent } from "./events";

export type RunStatus = "running" | "done" | "failed" | "stopped" | "skipped";

export type Run = {
  id: number;
  status: RunStatus;
  startedAt: number;
  finishedAt: number | null;
  /** 예약보다 얼마나 늦게 시작했나(ms). */
  lateMs: number;
  /** 바깥으로 나간 원문. */
  sentText: string;
  response: string | null;
  stderr: string | null;
  read: boolean;
};

export type Errand = {
  id: number;
  title: string;
  prompt: string;
  target: string;
  startAt: number;
  /** "" = 대화만 · "WebSearch" */
  allowedTools: string;
  /** run = 놓쳐도 늦게 실행 · skip = 건너뜀 */
  late: "run" | "skip";
  approved: boolean;
  run: Run | null;
};

export type ErrandInput = Pick<Errand, "prompt" | "startAt" | "allowedTools" | "late">;

/** 러스트 `errands.rs` 의 TOOL_CHOICES 와 같아야 한다. */
export const TOOL_OPTIONS: { value: string; label: string }[] = [
  { value: "", label: "대화만" },
  { value: "WebSearch", label: "웹 검색" },
];
export const LATE_OPTIONS: { value: "run" | "skip"; label: string }[] = [
  { value: "run", label: "늦게라도 실행" },
  { value: "skip", label: "건너뜀" },
];

export const errandApi = {
  list: (from: Date, to: Date) => invoke<Errand[]>("list_errands", { from: from.getTime(), to: to.getTime() }),
  create: (input: ErrandInput) => invoke<Errand>("create_errand", { input }),
  update: (id: number, input: ErrandInput) => invoke<Errand>("update_errand", { id, input }),
  remove: (id: number) => invoke<void>("delete_errand", { id }),
  restore: (id: number) => invoke<Errand>("restore_errand", { id }),
  runNow: (id: number) => invoke<void>("run_errand_now", { id }),
  stop: (runId: number) => invoke<void>("stop_run", { runId }),
  markRead: (runId: number) => invoke<void>("mark_run_read", { runId }),
};

export type ErrandState = "waiting" | "running" | "done" | "failed" | "stopped" | "skipped";

export function stateOf(e: Errand): ErrandState {
  return e.run ? e.run.status : "waiting";
}

/** 답이 왔는데 아직 안 읽음 — 점이 액센트로 채워진다. */
export function unread(e: Errand): boolean {
  return e.run?.status === "done" && !e.run.read;
}

export function stateLabel(e: Errand): string {
  switch (stateOf(e)) {
    case "waiting":
      return "대기";
    case "running":
      return "도는 중";
    case "done":
      return unread(e) ? "답이 왔어요" : "답";
    case "failed":
      return "실패";
    case "stopped":
      return "멈춤";
    case "skipped":
      return "건너뜀";
  }
}

/** «3분 늦게 실행됨». 1분 안쪽은 늦었다고 하지 않는다(30초 주기로 도는 일꾼의 오차). */
export function lateLabel(ms: number): string | null {
  const min = Math.round(ms / 60_000);
  if (min < 1) return null;
  return min < 60 ? `${min}분 늦게 실행됨` : `${Math.floor(min / 60)}시간${min % 60 ? ` ${min % 60}분` : ""} 늦게 실행됨`;
}

export function durationLabel(run: Run): string | null {
  if (run.finishedAt === null) return null;
  const s = Math.max(0, Math.round((run.finishedAt - run.startedAt) / 1000));
  return s < 60 ? `${s}초 걸림` : `${Math.floor(s / 60)}분 ${s % 60}초 걸림`;
}

/** 빠른 입력의 «클로드한테» — 부탁 표시. 글에서 그 말만 걷어 낸 나머지와 부탁인지를 돌려준다(날짜는 러스트 파서가 읽는다). */
const TARGET = /(?:클로드|Claude|claude)\s*(?:코드\s*)?(?:한테|에게|에다가?|께)(?=\s|$|[,.])\s*/u;
export function splitTarget(text: string): { text: string; errand: boolean } {
  if (!TARGET.test(text)) return { text, errand: false };
  return { text: text.replace(TARGET, "").replace(/\s{2,}/g, " ").trim(), errand: true };
}

export type Row = { kind: "event"; e: OuroEvent } | { kind: "errand"; r: Errand };

/** 그날의 줄 — 종일 일정이 위, 그다음 시각순(일정과 부탁을 섞는다). */
export function rowsOn(events: OuroEvent[], errands: Errand[], day: Date): Row[] {
  const start = startOfDay(day).getTime();
  const end = addDays(startOfDay(day), 1).getTime();
  const rows: Row[] = eventsOn(events, day).map((e) => ({ kind: "event", e }));
  for (const r of errands) if (r.startAt >= start && r.startAt < end) rows.push({ kind: "errand", r });
  const key = (x: Row) => (x.kind === "event" ? x.e : null);
  return rows.sort((a, b) => {
    const [ea, eb] = [key(a), key(b)];
    if (ea && eb) return byDayOrder(ea, eb);
    if (ea?.allDay) return -1;
    if (eb?.allDay) return 1;
    const t = (x: Row) => (x.kind === "event" ? (x.e.startAt ?? 0) : x.r.startAt);
    return t(a) - t(b) || (a.kind === "event" ? -1 : 1);
  });
}
