// 부탁 시트. 아직 안 돈 부탁(또는 새 부탁)은 «쓰기», 한 번이라도 돈 부탁은 «답 보기».
//
// 바깥으로 나가는 건 위 칸의 문장 **그대로**다 — 시트가 그렇게 말한다(PLAN §2-④: 무엇이 나가는지 부탁마다 보여 준다).
// 답은 글자로만 그린다(`whitespace-pre-wrap`, HTML·마크다운으로 해석하지 않는다) — 바깥에서 온 글은 명령도 마크업도 아니다.

import { useEffect, useRef, useState } from "react";
import { combine, hm, ymd } from "../lib/time";
import {
  durationLabel,
  LATE_OPTIONS,
  lateLabel,
  stateLabel,
  stateOf,
  TOOL_OPTIONS,
  type Errand,
  type ErrandInput,
} from "../lib/errands";
import { fmt } from "../lib/time";

export type ErrandDraft = {
  prompt: string;
  date: string;
  time: string;
  allowedTools: string;
  late: "run" | "skip";
};

export function blankErrandDraft(at: Date): ErrandDraft {
  return { prompt: "", date: ymd(at), time: hm(at), allowedTools: "", late: "run" };
}

export function errandInputToDraft(e: ErrandInput): ErrandDraft {
  const d = new Date(e.startAt);
  return { prompt: e.prompt, date: ymd(d), time: hm(d), allowedTools: e.allowedTools, late: e.late };
}

export const errandToDraft = (e: Errand): ErrandDraft => errandInputToDraft(e);

function toInput(d: ErrandDraft): ErrandInput | string {
  if (!d.prompt.trim()) return "부탁할 말을 적어 주세요";
  const at = combine(d.date, d.time);
  if (!at) return "없는 날짜·시각이에요 — 확인해 주세요";
  return { prompt: d.prompt, startAt: at.getTime(), allowedTools: d.allowedTools, late: d.late };
}

const field =
  "h-11 rounded-md bg-surface-sunken px-3 text-body-sm text-ink outline-none transition-shadow duration-100 focus:shadow-[inset_0_0_0_1.5px_var(--ink-secondary)]";
const ghost =
  "h-13 rounded-md bg-surface-sunken px-5 text-body-sm font-semibold transition-colors duration-100 active:bg-hairline disabled:opacity-40";
const primary =
  "h-13 flex-1 rounded-md bg-ink text-body-sm font-semibold text-canvas transition-opacity duration-100 active:opacity-80 disabled:opacity-40";

function toolsLabel(tools: string): string {
  return TOOL_OPTIONS.find((o) => o.value === tools)?.label ?? tools;
}

export function ErrandSheet({
  errand,
  initial,
  notice,
  proposal,
  onSave,
  onDelete,
  onRunNow,
  onStop,
  onClose,
}: {
  /** 이미 있는 부탁이면 그것(App 이 목록에서 늘 최신으로 넘긴다 — 도는 중 → 답이 오면 이 시트가 따라 바뀐다). 새 부탁이면 null. */
  errand: Errand | null;
  initial: ErrandDraft;
  notice?: string[];
  /** AI 의 제안을 고치는 시트 — 저장 = 승인(`approve_errand_proposal`). */
  proposal?: boolean;
  onSave: (input: ErrandInput) => Promise<string | null>;
  onDelete: () => void;
  onRunNow: () => Promise<string | null>;
  onStop: () => Promise<string | null>;
  onClose: () => void;
}) {
  const [d, setD] = useState<ErrandDraft>(initial);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);
  const viewing = !!errand?.run;
  // 고치는 중이면 «지금 실행» 은 막는다 — 실행은 저장된 문장으로 나가니 화면의 문장과 어긋난다(코덱스 개발 5).
  const dirty = JSON.stringify(d) !== JSON.stringify(initial);

  useEffect(() => {
    if (viewing) return;
    // 올라오는 모션이 끝난 뒤 포커스(EventSheet 와 같은 이유).
    const t = window.setTimeout(() => ref.current?.focus(), 320);
    return () => window.clearTimeout(t);
  }, [viewing]);

  const set = (patch: Partial<ErrandDraft>) => {
    setError(null);
    setD((cur) => ({ ...cur, ...patch }));
  };

  const guard = async (fn: () => Promise<string | null>) => {
    if (busy) return;
    setBusy(true);
    const err = await fn();
    setBusy(false);
    if (err) setError(err);
  };

  const submit = (ev?: React.FormEvent) => {
    ev?.preventDefault();
    const input = toInput(d);
    if (typeof input === "string") return setError(input);
    void guard(() => onSave(input));
  };

  const header = (title: string) => (
    <header className="flex h-12 shrink-0 items-center justify-between px-5">
      <button type="button" onClick={onClose} className="text-body-sm text-ink-secondary active:text-ink">
        {viewing ? "닫기" : "취소"}
      </button>
      <span className="text-label font-semibold">{title}</span>
      <span className="w-8" />
    </header>
  );

  const errorLine = error && (
    <p role="alert" className="text-caption text-danger">
      {error}
    </p>
  );

  // ── 답 보기 ──
  if (errand?.run) {
    const run = errand.run;
    const st = stateOf(errand);
    const late = lateLabel(run.lateMs);
    const meta = [
      fmt.time.format(run.startedAt),
      durationLabel(run),
      late,
    ].filter(Boolean);
    return (
      <section className="sheet-in absolute inset-x-0 bottom-0 top-3 flex flex-col rounded-t-xl bg-surface" aria-label="부탁과 답">
        {header("부탁")}
        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-5 pb-4">
          <div>
            <p className="text-body font-semibold">{errand.title}</p>
            <p className={`num mt-1 text-caption ${st === "failed" ? "text-danger" : st === "done" ? "text-accent" : "text-ink-muted"}`}>
              {stateLabel(errand)}
              {meta.length > 0 && ` · ${meta.join(" · ")}`}
            </p>
          </div>

          {st === "running" && <p className="text-body-sm text-ink-muted">Claude Code 가 일하는 중이에요…</p>}
          {st === "done" && (
            <div className="select-text whitespace-pre-wrap break-words text-body-sm text-ink">{run.response}</div>
          )}
          {(st === "failed" || st === "skipped") && (
            <p className="select-text whitespace-pre-wrap break-words text-body-sm text-ink-secondary">
              {run.stderr ?? "이유를 남기지 못했어요"}
            </p>
          )}
          {st === "stopped" && <p className="text-body-sm text-ink-muted">중단했어요.</p>}

          <details className="group">
            <summary className="cursor-pointer text-caption text-ink-muted marker:content-none">
              보낸 원문 · 도구 {toolsLabel(errand.allowedTools)}
            </summary>
            <p className="select-text mt-2 whitespace-pre-wrap break-words rounded-md bg-surface-sunken px-3 py-2.5 text-caption text-ink-secondary">
              {run.sentText}
            </p>
          </details>
        </div>
        <footer className="flex shrink-0 flex-col gap-2 px-5 pb-5">
          {errorLine}
          <div className="flex gap-2">
            {st === "running" ? (
              <button type="button" disabled={busy} onClick={() => void guard(onStop)} className={`${primary}`}>
                중단
              </button>
            ) : (
              <>
                <button type="button" onClick={onDelete} className={`${ghost} text-danger`}>
                  삭제
                </button>
                <button type="button" disabled={busy} onClick={() => void guard(onRunNow)} className={primary}>
                  다시 실행
                </button>
              </>
            )}
          </div>
        </footer>
      </section>
    );
  }

  // ── 쓰기 ──
  return (
    <form
      onSubmit={submit}
      onKeyDown={(e) => {
        if (e.key === "Enter" && e.metaKey) submit();
      }}
      className="sheet-in absolute inset-x-0 bottom-0 top-3 flex flex-col rounded-t-xl bg-surface"
      aria-label={proposal ? "제안 고치기" : errand ? "부탁 고치기" : "새 부탁"}
    >
      {header(proposal ? "제안 고치기" : errand ? "부탁" : "새 부탁")}
      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-5 pb-4">
        {notice?.map((n) => (
          <p key={n} className="text-caption text-ink-muted">
            {n}
          </p>
        ))}
        <textarea
          ref={ref}
          value={d.prompt}
          onChange={(e) => set({ prompt: e.target.value })}
          placeholder="Claude Code 에게 부탁할 말"
          rows={5}
          maxLength={4000}
          className="min-h-32 resize-none rounded-md bg-surface-sunken px-3 py-2.5 text-body-sm text-ink outline-none transition-shadow duration-100 placeholder:text-ink-muted focus:shadow-[inset_0_0_0_1.5px_var(--ink-secondary)]"
        />
        <p className="-mt-1 text-micro text-ink-muted">위 글이 그대로 Claude Code 로 나가요. 일정 내용은 따로 붙지 않아요.</p>

        <div className="grid grid-cols-[2.5rem_minmax(0,1fr)_8.5rem] items-center gap-2">
          <span className="text-caption text-ink-muted">때</span>
          <input type="date" value={d.date} onChange={(e) => set({ date: e.target.value })} className={`${field} num`} />
          <input type="time" value={d.time} onChange={(e) => set({ time: e.target.value })} className={`${field} num`} />
        </div>

        <label className="grid grid-cols-[2.5rem_1fr] items-center gap-2">
          <span className="text-caption text-ink-muted">도구</span>
          <select value={d.allowedTools} onChange={(e) => set({ allowedTools: e.target.value })} className={field}>
            {TOOL_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
        <label className="grid grid-cols-[2.5rem_1fr] items-center gap-2">
          <span className="text-caption text-ink-muted">놓치면</span>
          <select value={d.late} onChange={(e) => set({ late: e.target.value as "run" | "skip" })} className={field}>
            {LATE_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
      </div>

      <footer className="flex shrink-0 flex-col gap-2 px-5 pb-5">
        {errorLine}
        <div className="flex gap-2">
          {errand && (
            <>
              <button type="button" onClick={onDelete} className={`${ghost} text-danger`}>
                삭제
              </button>
              <button
                type="button"
                disabled={busy || dirty}
                title={dirty ? "고친 내용을 먼저 저장해 주세요" : undefined}
                onClick={() => void guard(onRunNow)}
                className={`${ghost} text-ink`}
              >
                지금 실행
              </button>
            </>
          )}
          <button type="submit" disabled={busy} className={primary}>
            {proposal ? "승인" : errand ? "저장" : "만들기"}
          </button>
        </div>
      </footer>
    </form>
  );
}
