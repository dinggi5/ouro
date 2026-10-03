// AI 가 MCP 로 낸 일정 제안 — 러스트 `store.rs` 의 Proposal 과 그 문. 제안은 사람이 «넣기» 를 눌러야 일정이 된다
// (`approve_proposal` 이 유일한 길, 소켓엔 이 문이 없다 — mcp.rs).

import { invoke } from "@tauri-apps/api/core";
import type { Errand, ErrandInput } from "./errands";
import type { EventInput, OuroEvent } from "./events";
import { dayLabel } from "./quick";
import { addDays, fmt, parseYmd, sameDay } from "./time";

export type Proposal = {
  id: number;
  /** MCP 클라이언트가 스스로 밝힌 이름 — 믿지 않고 보여 주기만 한다. */
  client: string;
  event: EventInput;
  status: "pending" | "approved" | "rejected" | "expired";
  createdAt: number;
  /** 겹치는 일정 제목(비공개는 «비공개 일정»). */
  conflicts: string[];
};

export const proposalApi = {
  list: () => invoke<Proposal[]>("list_proposals"),
  /** input = 시트에서 고친 값(없으면 제안 그대로). */
  approve: (id: number, input?: EventInput) => invoke<OuroEvent>("approve_proposal", { id, input: input ?? null }),
  reject: (id: number) => invoke<void>("reject_proposal", { id }),
  /** 지금 붙어 있는 AI(MCP 클라이언트 이름) — 사이드카가 15초마다 알린다. */
  clients: () => invoke<string[]>("mcp_clients"),
};

/** AI 가 낸 부탁 제안 — 러스트 `errands.rs` 의 ErrandProposal. 사람이 «승인» 해야 부탁이 되고 그제야 돈다(`approve_errand_proposal` 이 유일한 길). */
export type ErrandProposal = {
  id: number;
  client: string;
  /** 보낼 원문 — 승인하면 이 글이 그대로 Claude Code 로 나간다(고쳐서 승인하면 고친 글). */
  prompt: string;
  startAt: number;
  allowedTools: string;
  late: "run" | "skip";
  /** 어느 실행의 답에서 이어졌나(0 = 사람이 만든 뿌리). 최대 3. */
  depth: number;
  status: "pending" | "approved" | "rejected" | "expired";
  createdAt: number;
};

export const errandProposalApi = {
  list: () => invoke<ErrandProposal[]>("list_errand_proposals"),
  /** input = 시트에서 고친 값(없으면 제안 그대로). */
  approve: (id: number, input?: ErrandInput) => invoke<Errand>("approve_errand_proposal", { id, input: input ?? null }),
  reject: (id: number) => invoke<void>("reject_errand_proposal", { id }),
};

/** 카드 큐의 한 장 — 일정 제안과 부탁 제안을 도착 순서대로 섞는다. */
export type Card = { kind: "event"; p: Proposal } | { kind: "errand"; p: ErrandProposal };

export function queueOf(events: Proposal[], errands: ErrandProposal[]): Card[] {
  const all: Card[] = [
    ...events.map((p): Card => ({ kind: "event", p })),
    ...errands.map((p): Card => ({ kind: "errand", p })),
  ];
  return all.sort((a, b) => a.p.createdAt - b.p.createdAt || (a.kind === "event" ? -1 : 1) || a.p.id - b.p.id);
}

const CLIENTS: Record<string, string> = {
  "claude-code": "Claude Code",
  "claude-ai": "Claude",
  "codex-mcp-client": "Codex",
  codex: "Codex",
};

/** «Claude Code» 처럼 사람이 아는 이름. 모르면 받은 글 그대로(러스트가 길이·제어 문자를 이미 걸렀다). */
export function clientLabel(c: string): string {
  // 자기 속성만 — `__proto__` 같은 이름이 Object.prototype 을 돌려줘 렌더를 터뜨리지 않게(코덱스 개발 6).
  return Object.prototype.hasOwnProperty.call(CLIENTS, c) ? CLIENTS[c] : c;
}

/** 부탁 제안 시트용 입력 모양. */
export function errandInputOf(p: ErrandProposal): ErrandInput {
  return { prompt: p.prompt, startAt: p.startAt, allowedTools: p.allowedTools, late: p.late };
}

/** «내일 · 오전 9:00» — 부탁은 시각 하나다. */
export function errandWhen(startAt: number, today: Date): string {
  const s = new Date(startAt);
  return `${dayLabel(s, today)} · ${fmt.time.format(s)}`;
}

/** 시트로 열기 위한 일정 모양(아직 id 없음). */
export function asEvent(p: Proposal): OuroEvent {
  return { ...p.event, id: 0, origin: "mcp", updatedAt: p.createdAt };
}

/** «내일 · 10월 2일 금요일 · 오후 3:00 – 오후 4:00». 요일을 늘 보인다 — AI 가 «금요일» 을 날짜로 잘못 바꿨으면 여기서 보인다. */
export function proposalWhen(e: EventInput, today: Date): string {
  if (e.allDay) {
    const s = parseYmd(e.startDate ?? "");
    const x = parseYmd(e.endDate ?? "");
    if (!s || !x) return "";
    const last = addDays(x, -1);
    if (sameDay(s, last)) return `${dayLabel(s, today)} · 종일`;
    const days = Math.round((x.getTime() - s.getTime()) / 86_400_000);
    return `${dayLabel(s, today)} – ${fmt.monthDay.format(last)} · ${days}일`;
  }
  const s = new Date(e.startAt ?? 0);
  const x = new Date(e.endAt ?? 0);
  const end = sameDay(s, x) ? fmt.time.format(x) : `${fmt.monthDay.format(x)} ${fmt.time.format(x)}`;
  return `${dayLabel(s, today)} · ${fmt.time.format(s)} – ${end}`;
}
