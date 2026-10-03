// 업데이트 카드 — 제안 카드와 같은 틀로 목록 위에 뜬다. **노트를 보여 준 뒤 사람이 눌러야만** 깐다(update.rs).
// 업데이트는 AI 가 아니라 앱의 일이라 액센트를 안 쓴다(DESIGN: 색은 부탁·답만). 노트는 글자로만 그린다.

import type { UpdateState } from "../lib/update";

const CARD =
  "toast-in mx-5 mt-4 flex shrink-0 flex-col gap-1 rounded-lg bg-surface px-4 pt-3 pb-4 shadow-[0_2px_10px_rgba(0,27,55,0.10),0_3px_20px_rgba(2,32,71,0.05)] ring-1 ring-hairline";

export function UpdateCard({ u }: { u: UpdateState }) {
  if (!u.open) return null;
  if (!u.info) {
    // 사람이 확인을 눌렀는데 최신이거나, 확인이 실패했을 때.
    return (
      <section aria-label="업데이트" className={CARD}>
        <div className="flex items-center justify-between gap-3">
          <p className={`text-body-sm ${u.checking ? "text-ink-muted" : "text-ink"}`}>{u.checking ? "확인하는 중…" : u.message}</p>
          <button type="button" onClick={u.close} className="shrink-0 text-label text-ink-muted hover:text-ink-secondary">
            닫기
          </button>
        </div>
      </section>
    );
  }
  const pct = u.progress === null ? null : Math.round(u.progress * 100);
  return (
    <section aria-label="새 버전" className={CARD}>
      <p className="text-caption text-ink-muted">
        새 버전 · 지금 <span className="num">{u.info.currentVersion}</span>
      </p>
      <p className="num text-body-sm font-semibold text-ink">Ouro {u.info.version}</p>
      {u.info.notes && (
        <p className="select-text mt-1 max-h-40 overflow-y-auto whitespace-pre-wrap break-words rounded-md bg-surface-sunken px-3 py-2.5 text-caption text-ink">
          {u.info.notes}
        </p>
      )}
      {u.message && (
        <p role="alert" className="text-micro text-danger">
          {u.message}
        </p>
      )}
      <div className="mt-3 flex gap-2">
        <button
          type="button"
          disabled={u.installing}
          onClick={u.install}
          className="num h-10 flex-1 rounded-md bg-ink text-label font-semibold text-canvas transition-opacity duration-100 active:opacity-80 disabled:opacity-40"
        >
          {u.installing ? (pct === null ? "내려받는 중…" : `내려받는 중 ${pct}%`) : "설치하고 다시 켜기"}
        </button>
        <button
          type="button"
          disabled={u.installing}
          onClick={u.close}
          className="h-10 rounded-md px-3 text-label text-ink-muted transition-colors duration-100 hover:text-ink-secondary disabled:opacity-40"
        >
          나중에
        </button>
      </div>
    </section>
  );
}

/** 저절로 찾았을 때 아래에 한 줄 — 누르면 카드를 펼친다. */
export function UpdateLine({ u }: { u: UpdateState }) {
  if (!u.info || u.open) return null;
  return (
    <button
      type="button"
      onClick={u.show}
      className="num flex shrink-0 items-center px-5 pt-2 text-left text-micro text-ink-secondary hover:text-ink"
    >
      새 버전 {u.info.version} · 보기
    </button>
  );
}
