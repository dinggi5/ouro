// iCloud 동기화 카드 — 업데이트 카드와 같은 틀(앱의 일이라 액센트 없음, DESIGN). 메뉴바 «iCloud 동기화…» 로만 열린다.
// 한 장에 셋만: 켬/끔 · 지금 상태 한 줄 · 예약 부탁을 돌리는 맥.

import { agoText, type SyncState } from "../lib/sync";

const CARD =
  "toast-in mx-5 mt-4 flex shrink-0 flex-col gap-1 rounded-lg bg-surface px-4 pt-3 pb-4 shadow-[0_2px_10px_rgba(0,27,55,0.10),0_3px_20px_rgba(2,32,71,0.05)] ring-1 ring-hairline";

function stateLine(s: NonNullable<SyncState["status"]>, now: number): string {
  if (!s.on) return "꺼져 있어요 — 일정은 이 맥에만 있어요";
  if (s.error) return s.error;
  if (s.pending > 0) return `보내는 중 · ${s.pending}개 남음`;
  if (s.lastSync) return `맞췄어요 · ${agoText(s.lastSync, now)}`;
  return "처음 맞추는 중…";
}

export function SyncCard({ sync, hidden, now }: { sync: SyncState; hidden: boolean; now: number }) {
  const s = sync.status;
  if (!sync.open || hidden || !s) return null;
  const bad = s.on && !!s.error;
  return (
    <section aria-label="iCloud 동기화" className={CARD}>
      <div className="flex items-center justify-between gap-3">
        <p className="text-body-sm font-semibold text-ink">iCloud 동기화</p>
        <button type="button" onClick={sync.close} className="shrink-0 text-label text-ink-muted hover:text-ink-secondary">
          닫기
        </button>
      </div>
      <p className="text-caption text-ink-secondary">
        일정·부탁·답이 내 iCloud 로 내 기기끼리 오가요. 제목과 글은 종단간 암호화돼 애플도 못 읽어요.
      </p>
      <p role="status" className={`num mt-1 text-caption ${bad ? "text-danger" : "text-ink-muted"}`}>
        {stateLine(s, now)}
      </p>
      {s.on && (
        <div className="mt-2 flex items-center justify-between gap-3 rounded-md bg-surface-sunken px-3 py-2.5">
          <p className="min-w-0 text-caption text-ink">
            <span className="text-ink-muted">예약 부탁을 돌리는 맥 · </span>
            {s.runnerIsMe ? "이 맥" : (s.runnerName ?? "아직 없음")}
          </p>
          {!s.runnerIsMe && (
            <button
              type="button"
              disabled={sync.busy}
              onClick={sync.makeRunner}
              className="shrink-0 text-label font-semibold text-ink hover:text-ink-secondary disabled:opacity-40"
            >
              이 맥으로
            </button>
          )}
        </div>
      )}
      {sync.message && (
        <p role="alert" className="text-micro text-danger">
          {sync.message}
        </p>
      )}
      <button
        type="button"
        disabled={sync.busy}
        onClick={sync.toggle}
        className={`mt-3 h-10 rounded-md text-label font-semibold transition-opacity duration-100 active:opacity-80 disabled:opacity-40 ${
          s.on ? "bg-surface-sunken text-ink" : "bg-ink text-canvas"
        }`}
      >
        {s.on ? "끄기" : "켜기"}
      </button>
    </section>
  );
}

/** 켜져 있는데 문제가 있을 때만 아래 한 줄 — 누르면 카드. */
export function SyncLine({ sync }: { sync: SyncState }) {
  const s = sync.status;
  if (!s?.on || !s.error || sync.open) return null;
  return (
    <button type="button" onClick={sync.show} className="flex shrink-0 items-center px-5 pt-2 text-left text-micro text-danger">
      {s.error} · 보기
    </button>
  );
}
