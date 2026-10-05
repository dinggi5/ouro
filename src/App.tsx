// 앱 루트 — 오늘/주/월 세 보기 + 일정 시트 + 빠른 입력 + 되돌리기 토스트.
//
// 상태는 둘뿐이다: 보기(view) 와 커서(cursor, 고른 날). 세 보기가 같은 커서를 공유해서, 월에서 고른 날로
// «오늘» 탭을 누르면 그날 목록이 뜬다. 일정은 보기마다 필요한 창만 러스트에서 불러온다 — 캐시를 두지 않는다
// (쓰는 곳이 앱 하나라 목록이 틀릴 일이 없고, DB 가 로컬이라 한 번 읽는 데 1ms 도 안 걸린다).

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";
import { DayList } from "./components/DayList";
import { MonthGrid, monthGridRange } from "./components/MonthGrid";
import { blankDraft, EventSheet, fromEvent, toInput, type Draft } from "./components/EventSheet";
import { UpdateCard, UpdateLine } from "./components/UpdateCard";
import { SyncCard, SyncLine } from "./components/SyncCard";
import { ErrandProposalCard } from "./components/ErrandProposalCard";
import { ProposalCard } from "./components/ProposalCard";
import { QuickBar } from "./components/QuickBar";
import { blankErrandDraft, errandInputToDraft, errandToDraft, ErrandSheet, type ErrandDraft } from "./components/ErrandSheet";
import { errandApi, rowsOn, type Briefing, type Errand, type ErrandInput, type QuickErrand } from "./lib/errands";
import {
  asEvent,
  clientLabel,
  errandInputOf,
  errandProposalApi,
  proposalApi,
  queueOf,
  type ErrandProposal,
  type Proposal,
} from "./lib/proposals";
import { api, errorText, type EventInput, type OuroEvent } from "./lib/events";
import { canDirect, quickApi, quickToDraft, type QuickDraft } from "./lib/quick";
import { useUpdate } from "./lib/update";
import { useSync } from "./lib/sync";
import {
  addDays,
  addMonths,
  combine,
  fmt,
  msUntilMidnight,
  hm,
  nextHour,
  parseYmd,
  sameDay,
  startOfDay,
  startOfWeek,
  ymd,
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

/** fromQuick = 빠른 입력에서 열린 시트 — 저장하면 입력칸을 비운다(취소하면 친 글이 남는다).
 *  proposalId = AI 제안을 고쳐서 넣는 시트 — 저장이 `approve_proposal` 로 간다. */
type Sheet = {
  draft: Draft;
  editing: OuroEvent | null;
  key: number;
  fromQuick?: boolean;
  notice?: string[];
  proposalId?: number;
};
/** 부탁 시트. id = 이미 있는 부탁(목록에서 늘 최신으로 찾는다), null = 새 부탁. */
/** proposalId = AI 부탁 제안을 고쳐서 승인하는 시트 — 저장이 `approve_errand_proposal` 로 간다. */
type ErrandSheetState = {
  id: number | null;
  draft: ErrandDraft;
  key: number;
  fromQuick?: boolean;
  notice?: string[];
  proposalId?: number;
};
type Toast = { text: string; undo?: () => void; key: number };

function App() {
  const now = useNow();
  const today = startOfDay(now);
  const [view, setView] = useState<View>("day");
  const [cursor, setCursor] = useState<Date>(today);
  const [events, setEvents] = useState<OuroEvent[]>([]);
  const [fatal, setFatal] = useState<string | null>(null);
  const [backupError, setBackupError] = useState<string | null>(null);
  const [sheet, setSheet] = useState<Sheet | null>(null);
  const [errands, setErrands] = useState<Errand[]>([]);
  const [errandSheet, setErrandSheet] = useState<ErrandSheetState | null>(null);
  const [toast, setToast] = useState<Toast | null>(null);
  const sheetKey = useRef(0);
  const [quickText, setQuickText] = useState("");
  const [proposals, setProposals] = useState<Proposal[]>([]);
  const [errandProposals, setErrandProposals] = useState<ErrandProposal[]>([]);
  const [proposalBusy, setProposalBusy] = useState(false);
  const [aiClients, setAiClients] = useState<string[]>([]);
  const [brief, setBrief] = useState<Briefing | null>(null);
  const quickRef = useRef<HTMLInputElement>(null);
  const update = useUpdate();
  const sync = useSync();
  const [addMenu, setAddMenu] = useState(false);

  // 자정이 지나면 «오늘» 에 머물던 커서도 따라 넘어간다. 다른 날을 보고 있었다면 그대로 둔다.
  const lastToday = useRef(today);
  useEffect(() => {
    if (!sameDay(lastToday.current, today)) {
      setCursor((c) => (sameDay(c, lastToday.current) ? today : c));
      lastToday.current = today;
    }
  }, [today]);

  // 백업 경고는 따로 읽는다 — 이걸 못 읽었다고 일정 목록을 가리면 안 된다. 백업은 다른 스레드에서 자정·재시도로 돌므로
  // 팝오버가 떠 있는 동안에도 1분마다(now 가 바뀔 때) 다시 본다(코덱스 개발 2 2차).
  const nowMinute = Math.floor(now.getTime() / 60_000);
  useEffect(() => {
    api.backupError().then(setBackupError, () => {});
  }, [nowMinute]);
  // 브리핑의 «다음 일정·빈 시간» 은 시각에 따라 바뀐다 — 팝오버가 떠 있어도 1분마다 다시 센다(코덱스 개발 7).
  useEffect(() => {
    errandApi.briefing().then(setBrief, () => {});
  }, [nowMinute]);

  // 붙어 있는 AI. 붙거나 나가면 러스트가 «mcp-changed» 를 보낸다. 소식이 끊겨 조용히 빠지는 건 이벤트가 없어서
  // 1분마다·다시 보일 때 한 번 더 읽는다.
  const loadClients = useCallback(() => {
    proposalApi.clients().then(setAiClients, () => {});
  }, []);
  useEffect(loadClients, [loadClients, nowMinute]);
  useEffect(() => {
    window.addEventListener("focus", loadClients);
    let unlisten: (() => void) | null = null;
    let alive = true;
    listen("mcp-changed", loadClients).then(
      (u) => (alive ? (unlisten = u) : u()),
      () => {},
    );
    return () => {
      alive = false;
      window.removeEventListener("focus", loadClients);
      unlisten?.();
    };
  }, [loadClients]);

  const [from, to] = useMemo(() => rangeOf(view, cursor), [view, cursor]);
  // 늦게 온 답은 버린다 — 다른 날로 넘긴 뒤 앞 날의 느린 답이 목록을 덮지 않게(코덱스 개발 4). 제안도 같은 규칙.
  const eventsSeq = useRef(0);
  const reload = useCallback(async () => {
    const seq = ++eventsSeq.current;
    try {
      const [list, errs, b] = await Promise.all([api.list(from, to), errandApi.list(from, to), errandApi.briefing().catch(() => null)]);
      if (seq !== eventsSeq.current) return;
      setEvents(list);
      setErrands(errs);
      setBrief(b);
      setFatal(null);
    } catch (e) {
      if (seq === eventsSeq.current) setFatal(errorText(e));
    }
  }, [from, to]);

  useEffect(() => {
    void reload();
    // 다시 보일 때 새로 읽는다 — 개발 4 부터는 MCP 로도 일정이 바뀐다.
    const onFocus = () => void reload();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [reload]);

  // 부탁이 시작하거나 끝나면 러스트가 «errands-changed» 를 보낸다 — 점(◯ ◔ ●)이 스스로 바뀐다.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let alive = true;
    listen("errands-changed", () => void reload()).then(
      (u) => (alive ? (unlisten = u) : u()),
      () => {},
    );
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [reload]);

  // AI 제안. 새 제안이 오면 러스트가 팝오버를 띄우고 «proposals-changed» 를 보낸다 — 이미 떠 있던 팝오버엔 focus 가 안 오니 이벤트로 듣는다.
  const proposalsSeq = useRef(0);
  const errandProposalsSeq = useRef(0);
  const loadProposals = useCallback(() => {
    const seq = ++proposalsSeq.current;
    proposalApi.list().then((list) => {
      if (seq === proposalsSeq.current) setProposals(list);
    }, () => {});
    const eseq = ++errandProposalsSeq.current;
    errandProposalApi.list().then((list) => {
      if (eseq === errandProposalsSeq.current) setErrandProposals(list);
    }, () => {});
  }, []);
  useEffect(() => {
    loadProposals();
    window.addEventListener("focus", loadProposals);
    let unlisten: (() => void) | null = null;
    let alive = true;
    listen("proposals-changed", loadProposals).then(
      (u) => (alive ? (unlisten = u) : u()),
      () => {},
    );
    return () => {
      alive = false;
      window.removeEventListener("focus", loadProposals);
      unlisten?.();
    };
  }, [loadProposals]);

  const showToast = useCallback((text: string, undo?: () => void) => {
    setToast({ text, undo, key: Date.now() });
  }, []);
  useEffect(() => {
    if (!toast) return;
    const t = window.setTimeout(() => setToast(null), 4000);
    return () => window.clearTimeout(t);
  }, [toast]);

  /** 업데이트를 받는 중이면 곧 다시 켜진다 — 고치기 시트도 열지 않는다(쓰던 초안이 사라진다, 코덱스 개발 8 P1·개발 10). */
  const blockedByUpdate = () => {
    if (update.installing) showToast("업데이트를 설치하는 중이에요");
    return update.installing;
  };

  const openErrand = (e: Errand) => {
    if (blockedByUpdate()) return;
    setErrandSheet({ id: e.id, draft: errandToDraft(e), key: ++sheetKey.current });
    // 열어 본 답은 «읽음» — 점이 액센트에서 회색으로.
    if (e.run?.status === "done" && !e.run.read) {
      errandApi.markRead(e.run.id).then(() => void reload(), () => {});
    }
  };

  const openEdit = (e: OuroEvent) => {
    if (blockedByUpdate()) return;
    setSheet({ draft: fromEvent(e), editing: e, key: ++sheetKey.current });
  };

  /** 만든 날로 커서를 옮긴다 — 다른 날에 만들었는데 목록에 안 보이면 «저장이 안 됐나» 한다. */
  const reveal = (saved: OuroEvent) => {
    const day = saved.allDay ? parseYmd(saved.startDate ?? "") : new Date(saved.startAt ?? 0);
    if (day && !sameDay(day, cursor)) setCursor(startOfDay(day));
  };

  const save = async (input: EventInput): Promise<string | null> => {
    try {
      const saved =
        sheet?.proposalId !== undefined
          ? await proposalApi.approve(sheet.proposalId, input)
          : sheet?.editing
            ? await api.update(sheet.editing.id, input)
            : await api.create(input);
      if (sheet?.proposalId !== undefined) loadProposals();
      if (sheet?.fromQuick) setQuickText("");
      // 저장한 그 시트만 닫는다 — 응답이 늦는 사이 닫고 새 시트를 열었으면 그건 두고(코덱스 개발 8).
      const savedKey = sheet?.key;
      setSheet((cur) => (cur?.key === savedKey ? null : cur));
      if (!sheet?.editing) reveal(saved);
      await reload();
      return null;
    } catch (e) {
      return errorText(e);
    }
  };

  const openSheetErrand = errandSheet?.id != null ? (errands.find((x) => x.id === errandSheet.id) ?? null) : null;
  // 열어 둔 부탁이 지워졌거나 창 밖으로 나갔으면 시트를 닫는다.
  useEffect(() => {
    if (errandSheet?.id != null && !openSheetErrand) setErrandSheet(null);
  }, [errandSheet, openSheetErrand]);

  const saveErrand = async (input: ErrandInput): Promise<string | null> => {
    try {
      const saved =
        errandSheet?.proposalId !== undefined
          ? await errandProposalApi.approve(errandSheet.proposalId, input)
          : errandSheet?.id != null
            ? await errandApi.update(errandSheet.id, input)
            : await errandApi.create(input);
      if (errandSheet?.proposalId !== undefined) loadProposals();
      if (errandSheet?.fromQuick) setQuickText("");
      const savedKey = errandSheet?.key;
      setErrandSheet((cur) => (cur?.key === savedKey ? null : cur));
      const day = new Date(saved.startAt);
      if (!sameDay(day, cursor)) setCursor(startOfDay(day));
      await reload();
      return null;
    } catch (e) {
      return errorText(e);
    }
  };

  const removeErrand = async () => {
    const id = errandSheet?.id;
    if (id == null) return;
    try {
      await errandApi.remove(id);
      setErrandSheet(null);
      await reload();
      showToast("삭제했어요", async () => {
        setToast(null);
        try {
          await errandApi.restore(id);
          await reload();
        } catch (err) {
          showToast(errorText(err));
        }
      });
    } catch (err) {
      showToast(errorText(err));
    }
  };

  /** 시트의 «지금 실행»·«다시 실행»·«중단». 실패는 문장으로 시트 안에 보인다. */
  const errandAction = (fn: () => Promise<void>) => async (): Promise<string | null> => {
    try {
      await fn();
      await reload();
      return null;
    } catch (e) {
      return errorText(e);
    }
  };

  /** + 버튼 — 문장 없이 빈 시트로 바로 넣는다(개발 8 뒤 사장 피드백: 빠른 입력만 있으니 «AI 채팅만 되는 앱» 처럼 보였다).
   *  보고 있는 날이 기준이다(오늘이면 다음 정각, 다른 날이면 9시) — 빠른 입력의 «날짜 없는 입력» 과 같은 규칙. */
  const addNew = (kind: "event" | "errand") => {
    setAddMenu(false);
    // 시트가 이미 열려 있으면 겹쳐 열지 않는다(시트 뒤 + 로 탭 이동해 올 수 있다 — 코덱스 개발 8).
    if (sheet || errandSheet) return;
    // 업데이트를 받는 중이면 곧 다시 켜진다 — 쓰던 초안이 사라지니 새로 쓰기 시작하지 않게(코덱스 개발 8 P1).
    if (update.installing) {
      showToast("업데이트를 설치하는 중이에요");
      return;
    }
    const now = new Date();
    if (kind === "event") {
      setSheet({ draft: blankDraft(cursor, now), editing: null, key: ++sheetKey.current });
    } else {
      // 부탁은 지난 때로 잡으면 만들자마자 돈다(놓치면 «늦게라도 실행»). 지난 날을 보고 있어도 기본은 다음 정각 — 일정과 다른 점.
      const day9 = new Date(cursor.getFullYear(), cursor.getMonth(), cursor.getDate(), 9);
      const at = day9.getTime() > now.getTime() && !sameDay(cursor, now) ? day9 : nextHour(now);
      setErrandSheet({ id: null, draft: blankErrandDraft(at), key: ++sheetKey.current });
    }
  };

  /** 빠른 입력 확정. 날짜를 알아들었고 모양이 맞으면 바로 넣고, 아니면(또는 ⌘↩) 그 초안으로 시트를 연다. */
  const quickCommit = async (q: QuickDraft, detail: boolean, errand: QuickErrand | null) => {
    if (update.installing) {
      showToast("업데이트를 설치하는 중이에요");
      return;
    }
    if (errand) {
      // «클로드한테»·«코덱스한테» — 부탁. 문장은 날짜 말을 걷어 낸 제목이고, 날짜·시각은 파서가 읽은 대로(없으면 다음 정각)다. 늘 시트로.
      // «매일·평일·매주» 는 반복으로 — 첫 회차 날짜는 파서가 정한다.
      const blank = {
        ...blankErrandDraft(
          sameDay(cursor, new Date()) ? nextHour(new Date()) : new Date(cursor.getFullYear(), cursor.getMonth(), cursor.getDate(), 9),
          errand.target,
        ),
        repeat: errand.repeat,
      };
      const at = q.startDate && !q.allDay ? combine(q.startDate, q.startTime ?? "09:00") : null;
      const draft: ErrandDraft = at
        ? { ...blank, prompt: q.title, date: ymd(at), time: hm(at) }
        : { ...blank, prompt: q.title, ...(q.startDate ? { date: q.startDate } : {}) };
      const notice = [
        ...errand.warnings(q.warnings),
        ...(q.startDate ? [] : ["날짜를 못 찾아서 다음 정각으로 잡았어요"]),
      ];
      setErrandSheet({ id: null, draft, key: ++sheetKey.current, fromQuick: true, notice });
      return;
    }
    if (q.miss) quickApi.recordMiss(q.miss).catch(() => {});
    const draft = quickToDraft(q, blankDraft(cursor, new Date()));
    // 바로 넣지 않는 때: 날짜를 못 찾았다(fallback 은 추측이다), 파서가 경고했다(없는 날짜를 빼고 남은 시각·반복의 첫 번 등,
    // 쓴 것과 다른 일정이 된다 — 코덱스 개발 3). 둘 다 시트에서 사람이 보고 저장한다. 판단은 `canDirect` 하나.
    const input = detail || !canDirect(q) ? null : toInput(draft);
    if (!input || typeof input === "string") {
      setSheet({ draft, editing: null, key: ++sheetKey.current, fromQuick: true, notice: q.warnings });
      return;
    }
    try {
      const saved = await api.create(input);
      setQuickText("");
      reveal(saved);
      await reload();
      showToast("넣었어요", async () => {
        setToast(null);
        try {
          await api.remove(saved.id);
          await reload();
        } catch (err) {
          showToast(errorText(err));
        }
      });
    } catch (e) {
      showToast(errorText(e));
    }
  };

  /** 제안 «넣기» — 그대로 일정으로. 되돌리기 = 만든 일정을 지운다(제안은 «넣음» 으로 남는다, 기록이라서). */
  const approveProposal = async (p: Proposal) => {
    setProposalBusy(true);
    try {
      const saved = await proposalApi.approve(p.id);
      reveal(saved);
      await reload();
      showToast("넣었어요", async () => {
        setToast(null);
        try {
          await api.remove(saved.id);
          await reload();
        } catch (err) {
          showToast(errorText(err));
        }
      });
    } catch (e) {
      showToast(errorText(e));
    } finally {
      setProposalBusy(false);
      loadProposals();
    }
  };

  const rejectProposal = async (p: Proposal) => {
    setProposalBusy(true);
    try {
      await proposalApi.reject(p.id);
    } catch (e) {
      showToast(errorText(e));
    } finally {
      setProposalBusy(false);
      loadProposals();
    }
  };

  /** 부탁 제안 «승인» — 그대로. 되돌리기 = 만든 부탁을 지운다(제안은 «받음» 으로 남는다, 기록이라서). */
  const approveErrandProposal = async (p: ErrandProposal) => {
    setProposalBusy(true);
    try {
      const saved = await errandProposalApi.approve(p.id);
      const day = new Date(saved.startAt);
      if (!sameDay(day, cursor)) setCursor(startOfDay(day));
      await reload();
      showToast("부탁을 걸었어요", async () => {
        setToast(null);
        try {
          await errandApi.remove(saved.id);
          await reload();
        } catch (err) {
          showToast(errorText(err));
        }
      });
    } catch (e) {
      showToast(errorText(e));
    } finally {
      setProposalBusy(false);
      loadProposals();
    }
  };

  const rejectErrandProposal = async (p: ErrandProposal) => {
    setProposalBusy(true);
    try {
      await errandProposalApi.reject(p.id);
    } catch (e) {
      showToast(errorText(e));
    } finally {
      setProposalBusy(false);
      loadProposals();
    }
  };

  /** 답 시트의 «이어서 부탁» — 그 답의 대화를 잇는 새 부탁. 때는 «지금»(그대로 만들면 바로 돈다). */
  const followUp = (e: Errand) => {
    if (!e.run) return;
    const draft = { ...blankErrandDraft(new Date(), e.target), resumeRunId: e.run.id, resumeTitle: e.title, resumes: true };
    setErrandSheet({ id: null, draft, key: ++sheetKey.current, notice: ["때를 그대로 두면 만들자마자 돌아요."] });
  };

  const toggleMorning = async () => {
    if (!brief) return;
    try {
      await errandApi.setMorning(!brief.morning);
      setBrief({ ...brief, morning: !brief.morning });
      showToast(brief.morning ? "아침 브리핑 알림을 껐어요" : "매일 아침 8시에 알려 드려요");
    } catch (e) {
      showToast(errorText(e));
    }
  };

  const editErrandProposal = (p: ErrandProposal) =>
    !blockedByUpdate() &&
    setErrandSheet({
      id: null,
      draft: errandInputToDraft(errandInputOf(p)),
      key: ++sheetKey.current,
      proposalId: p.id,
      notice: [`${clientLabel(p.client)} 가 낸 제안이에요 — 글을 고치면 고친 글이 나가요. 승인해야 부탁이 돼요.`],
    });

  const editProposal = (p: Proposal) =>
    !blockedByUpdate() &&
    setSheet({ draft: fromEvent(asEvent(p)), editing: null, key: ++sheetKey.current, proposalId: p.id });

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

  // 키보드: ⌘W 닫기(창이 무테라 AppKit 이 안 준다, Kura 개발 58), ⌘N 빠른 입력으로, Esc 시트 닫기, ←/→ 넘기기, T 오늘.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const typing = e.target instanceof HTMLElement && e.target.closest("input, textarea, select");
      // + 메뉴가 열려 있으면 메뉴만 — Esc 로 닫고, ←/→·T 가 뒤의 날짜를 넘기지 않게(코덱스 개발 8).
      // 탭으로 입력칸에 가 있으면 메뉴를 닫고 글자는 그대로 칠 수 있게(코덱스 개발 8 2차).
      if (addMenu && typing) setAddMenu(false);
      else if (addMenu) {
        if (e.key === "Escape" || e.metaKey) {
          e.preventDefault();
          setAddMenu(false);
        } else if (e.key === "ArrowLeft" || e.key === "ArrowRight" || e.key === "t" || e.key === "T") {
          e.preventDefault();
        }
        return;
      }
      if (e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) {
        const k = e.key.toLowerCase();
        if (k === "w") {
          e.preventDefault();
          void invoke("hide_popover");
        } else if (k === "n" && !sheet && !errandSheet) {
          e.preventDefault();
          quickRef.current?.focus();
        }
        return;
      }
      if (e.key === "Escape" && (sheet || errandSheet)) {
        e.preventDefault();
        if (sheet) setSheet(null);
        else setErrandSheet(null);
        return;
      }
      if (sheet || errandSheet || typing || e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key === "ArrowLeft") step(-1);
      else if (e.key === "ArrowRight") step(1);
      else if (e.key === "t" || e.key === "T") setCursor(today);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [sheet, errandSheet, step, today, addMenu]);

  const t = title(view, cursor);
  const atToday = sameDay(cursor, today);
  const nowMs = now.getTime();
  // 일정 제안과 부탁 제안을 도착 순서대로 한 줄로 세운다 — 카드는 맨 앞 하나만.
  const queue = useMemo(() => queueOf(proposals, errandProposals), [proposals, errandProposals]);
  const top = queue[0];

  return (
    <main className="relative flex h-screen w-full flex-col overflow-hidden rounded-lg bg-canvas text-ink">
      {/* 시트가 떠 있으면 뒤는 손이 안 닿는다(inert) — 탭·클릭으로 제안 카드 «고치기» 등을 눌러 시트가 겹치지 않게(코덱스 개발 8).
          contents = 상자를 안 만들어 아래 줄들이 그대로 main 의 flex 에 선다. */}
      <div className="contents" inert={!!(sheet || errandSheet)}>
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
            <IconButton label="새로 만들기" onClick={() => setAddMenu((o) => !o)}>
              <path d="M8 3.5v9M3.5 8h9" />
            </IconButton>
            <IconButton label="이전" onClick={() => step(-1)}>
              <path d="M10 3.5 5.5 8l4.5 4.5" />
            </IconButton>
            <IconButton label="다음" onClick={() => step(1)}>
              <path d="M6 3.5 10.5 8 6 12.5" />
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

      {top?.kind === "event" && (
        <ProposalCard
          key={`e${top.p.id}`}
          proposal={top.p}
          total={queue.length}
          today={today}
          busy={proposalBusy}
          onApprove={() => void approveProposal(top.p)}
          onEdit={() => editProposal(top.p)}
          onReject={() => void rejectProposal(top.p)}
        />
      )}
      {top?.kind === "errand" && (
        <ErrandProposalCard
          key={`r${top.p.id}`}
          proposal={top.p}
          total={queue.length}
          today={today}
          nowMs={nowMs}
          busy={proposalBusy}
          onApprove={() => void approveErrandProposal(top.p)}
          onEdit={() => editErrandProposal(top.p)}
          onReject={() => void rejectErrandProposal(top.p)}
        />
      )}

      <UpdateCard u={update} hidden={!!top} />
      {/* 카드는 한 장만 — 제안·업데이트 카드가 떠 있으면 동기화 카드는 물러난다(640 창에서 버튼이 밀리지 않게). */}
      <SyncCard sync={sync} hidden={!!top || (update.open && !!update.info)} now={now.getTime()} />

      <section className="min-h-0 flex-1 overflow-y-auto px-5 pt-4 pb-5">
        {fatal ? (
          <Empty text={fatal} />
        ) : view === "day" ? (
          <>
            {atToday && brief && brief.parts.length > 0 && (
              <p className="num mb-3 flex items-baseline gap-2 text-caption text-ink-muted">
                <span className="min-w-0 flex-1">{brief.parts.join(" · ")}</span>
                <button
                  type="button"
                  onClick={() => void toggleMorning()}
                  title={brief.morning ? "매일 아침 8시에 이 요약을 알림으로 받아요" : "아침 알림이 꺼져 있어요"}
                  className="shrink-0 text-micro text-ink-muted hover:text-ink-secondary"
                >
                  {brief.morning ? "아침 알림 켬" : "아침 알림 끔"}
                </button>
              </p>
            )}
            <DayBody day={cursor} events={events} errands={errands} now={nowMs} onOpen={openEdit} onOpenErrand={openErrand} />
          </>
        ) : view === "week" ? (
          <WeekBody
            cursor={cursor}
            today={today}
            events={events}
            errands={errands}
            now={nowMs}
            onOpen={openEdit}
            onOpenErrand={openErrand}
            onPickDay={(d) => {
              setCursor(d);
              setView("day");
            }}
          />
        ) : (
          <>
            <MonthGrid cursor={cursor} today={today} events={events} errands={errands} onPick={setCursor} />
            <div className="mt-4">
              <DayBody day={cursor} events={events} errands={errands} now={nowMs} onOpen={openEdit} onOpenErrand={openErrand} />
            </div>
          </>
        )}
      </section>

      {aiClients.length > 0 && (
        <p className="flex shrink-0 items-center gap-1.5 px-5 pt-2 text-micro text-accent" title="MCP 로 이 캘린더에 붙어 있어요">
          <svg width="10" height="10" viewBox="0 0 12 12" aria-hidden="true" className="shrink-0">
            <circle cx="6" cy="6" r="4.5" fill="none" stroke="currentColor" strokeWidth="1.5" />
          </svg>
          <span className="truncate">{aiClients.map(clientLabel).join(" · ")} 연결됨</span>
        </p>
      )}

      <UpdateLine u={update} cardHidden={!!top} />
      <SyncLine sync={sync} />

      {backupError && (
        <p role="alert" className="shrink-0 truncate px-5 pt-2 text-micro text-danger" title={backupError}>
          {backupError}
        </p>
      )}

      <QuickBar
        inputRef={quickRef}
        text={quickText}
        onText={(t) => {
          setQuickText(t);
          // 다음 것을 치기 시작하면 «넣었어요» 토스트는 걷는다 — 새 카드를 가린다(되돌리기는 일정을 눌러 지우면 된다).
          if (t) setToast(null);
        }}
        // 하루·월 보기에서 오늘이 아닌 날을 보고 있으면 날짜 없는 입력은 그날로(주 보기엔 «고른 날» 이 안 보여 오늘 기준).
        base={view !== "week" && !atToday ? ymd(cursor) : null}
        today={today}
        onCommit={quickCommit}
      />
      </div>

      {sheet && (
        <>
          <div className="scrim-in absolute inset-0 bg-black/30" onClick={() => setSheet(null)} />
          <EventSheet
            key={sheet.key}
            initial={sheet.draft}
            editing={sheet.editing}
            notice={sheet.notice}
            heading={sheet.proposalId !== undefined ? "제안 고치기" : undefined}
            onSave={save}
            onDelete={() => void remove()}
            onClose={() => setSheet(null)}
          />
        </>
      )}

      {errandSheet && (
        <>
          <div className="scrim-in absolute inset-0 bg-black/30" onClick={() => setErrandSheet(null)} />
          <ErrandSheet
            key={errandSheet.key}
            errand={openSheetErrand}
            initial={errandSheet.draft}
            notice={errandSheet.notice}
            proposal={errandSheet.proposalId !== undefined}
            onSave={saveErrand}
            onDelete={() => void removeErrand()}
            onRunNow={errandAction(() => errandApi.runNow(errandSheet.id as number))}
            onStop={errandAction(() => errandApi.stop(openSheetErrand?.run?.id ?? 0))}
            onFollowUp={() => openSheetErrand && followUp(openSheetErrand)}
            onClose={() => setErrandSheet(null)}
          />
        </>
      )}

      {addMenu && (
        <>
          {/* 바깥을 누르면 닫힌다. 투명 — 메뉴 하나에 화면을 어둡게 하지 않는다. */}
          <div className="absolute inset-0" onClick={() => setAddMenu(false)} />
          <div
            role="menu"
            className="toast-in absolute top-16 right-5 flex w-36 flex-col rounded-md bg-surface p-1 shadow-[0_2px_10px_rgba(0,27,55,0.10),0_3px_20px_rgba(2,32,71,0.05)] ring-1 ring-hairline"
          >
            <button type="button" role="menuitem" autoFocus onClick={() => addNew("event")} className="h-10 rounded-sm px-3 text-left text-body-sm text-ink hover:bg-surface-sunken">
              일정
            </button>
            <button type="button" role="menuitem" onClick={() => addNew("errand")} className="flex h-10 items-center gap-2 rounded-sm px-3 text-left text-body-sm text-ink hover:bg-surface-sunken">
              <span>부탁</span>
              <span className="text-caption text-ink-muted">AI 에게</span>
            </button>
          </div>
        </>
      )}

      {toast && (
        <div
          key={toast.key}
          role="status"
          className="toast-in absolute inset-x-0 bottom-20 mx-auto flex w-fit items-center gap-4 rounded-pill bg-[rgba(25,31,40,0.92)] px-5 py-3 text-body-sm text-white"
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

function Empty({ text }: { text: string }) {
  return <p className="py-12 text-center text-body-sm text-ink-muted">{text}</p>;
}

function DayBody(props: {
  day: Date;
  events: OuroEvent[];
  errands: Errand[];
  now: number;
  onOpen: (e: OuroEvent) => void;
  onOpenErrand: (e: Errand) => void;
}) {
  const rows = rowsOn(props.events, props.errands, props.day);
  if (rows.length === 0) return <Empty text="비어 있어요" />;
  return <DayList day={props.day} rows={rows} now={props.now} onOpen={props.onOpen} onOpenErrand={props.onOpenErrand} />;
}

function WeekBody(props: {
  cursor: Date;
  today: Date;
  events: OuroEvent[];
  errands: Errand[];
  now: number;
  onOpen: (e: OuroEvent) => void;
  onOpenErrand: (e: Errand) => void;
  onPickDay: (d: Date) => void;
}) {
  const start = startOfWeek(props.cursor);
  const days = Array.from({ length: 7 }, (_, i) => addDays(start, i));
  return (
    <div className="flex flex-col gap-4">
      {days.map((d) => {
        const list = rowsOn(props.events, props.errands, d);
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
            {list.length > 0 && <DayList day={d} rows={list} now={props.now} onOpen={props.onOpen} onOpenErrand={props.onOpenErrand} />}
          </div>
        );
      })}
    </div>
  );
}

export default App;
