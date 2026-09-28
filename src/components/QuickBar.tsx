// 빠른 입력 한 줄 — 팝오버 맨 아래. 「내일 3시 치과」 를 치는 동안 위에 카드가 떠서 무엇이 들어갈지 보여 준다.
//
// ↩ = 바로 넣기(날짜를 알아들었을 때) / 날짜를 못 찾았으면 시트를 열어 사람이 고른다(PLAN §7 «추측하지 않는다»).
// ⌘↩ 또는 카드 누르기 = 시트로 열어 고쳐서 넣기. Esc = 비우기.
// 파싱은 글자마다 러스트에 묻는다(로컬이라 1ms 도 안 걸린다). 넣을 때는 마지막 글자까지 반영된 초안을 한 번 더 받는다.

import { useEffect, useRef, useState } from "react";
import { canDirect, quickApi, whenLabel, type QuickDraft } from "../lib/quick";

export function QuickBar({
  inputRef,
  text,
  onText,
  base,
  today,
  onCommit,
}: {
  inputRef: React.RefObject<HTMLInputElement | null>;
  /** 글은 App 이 쥔다 — 시트로 열었다가 취소하면 친 글이 남아 있어야 하고, 시트에서 저장하면 비워야 해서. */
  text: string;
  onText: (t: string) => void;
  /** 보고 있는 날 `YYYY-MM-DD`(오늘이 아니면) — 날짜 없는 입력이 그날로 간다. */
  base: string | null;
  today: Date;
  /** detail = 시트로 열기. 입력칸을 비우는 건 App 이 정한다(바로 넣었거나 시트에서 저장했을 때). */
  onCommit: (q: QuickDraft, detail: boolean) => Promise<void>;
}) {
  const [draft, setDraft] = useState<QuickDraft | null>(null);
  const busy = useRef(false);

  useEffect(() => {
    if (!text.trim()) {
      setDraft(null);
      return;
    }
    // 늦게 온 답이 새 글자의 답을 덮지 않게.
    let alive = true;
    quickApi.parse(text, base).then(
      (d) => alive && setDraft(d),
      () => {},
    );
    return () => {
      alive = false;
    };
  }, [text, base]);

  const commit = async (detail: boolean) => {
    if (busy.current || !text.trim()) return;
    busy.current = true;
    try {
      await onCommit(await quickApi.parse(text, base), detail);
    } catch {
      // 파싱은 실패하지 않는다(러스트가 언제나 초안을 준다). 저장 실패는 onCommit 이 토스트로 알린다.
    } finally {
      busy.current = false;
    }
  };

  const when = draft ? whenLabel(draft, today) : null;
  const direct = !!draft && canDirect(draft);

  return (
    <div className="relative shrink-0 px-5 pt-2 pb-5">
      <div aria-live="polite">
        {draft && text.trim() && (
          <button
            type="button"
            // 누르는 순간 입력칸의 포커스를 뺏지 않는다 — 시트가 닫히면 다시 이어 칠 수 있게.
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => void commit(true)}
            className="toast-in absolute inset-x-5 bottom-full flex flex-col gap-1 rounded-md bg-surface px-4 py-3 text-left shadow-[0_2px_10px_rgba(0,27,55,0.10),0_3px_20px_rgba(2,32,71,0.05)] ring-1 ring-hairline"
          >
            <span className={`truncate text-body-sm font-semibold ${draft.title ? "text-ink" : "text-ink-muted"}`}>
              {draft.title || "제목 없음"}
            </span>
            <span className="num text-caption text-ink-secondary">{when ?? "날짜를 못 찾았어요"}</span>
            {draft.warnings.map((w) => (
              <span key={w} className="text-caption text-ink-muted">
                {w}
              </span>
            ))}
            <span className="mt-1 text-micro text-ink-muted">{direct ? "↩ 넣기 · ⌘↩ 고쳐서 넣기" : "↩ 확인하고 넣기"}</span>
          </button>
        )}
      </div>
      <input
        ref={inputRef}
        value={text}
        onChange={(e) => onText(e.target.value)}
        onKeyDown={(e) => {
          // 한글 조합 중의 ↩ 는 글자를 끝내는 키다 — 넣기로 받지 않는다(WebKit 은 keyCode 229 로 온다).
          if (e.nativeEvent.isComposing || e.keyCode === 229) return;
          if (e.key === "Enter") {
            e.preventDefault();
            void commit(e.metaKey);
          } else if (e.key === "Escape" && text) {
            e.preventDefault();
            onText("");
          }
        }}
        placeholder="새 일정 — 내일 3시 치과"
        maxLength={200}
        aria-label="빠른 입력"
        className="h-11 w-full rounded-md bg-surface-sunken px-4 text-body-sm text-ink outline-none transition-shadow duration-100 placeholder:text-ink-muted focus:shadow-[inset_0_0_0_1.5px_var(--ink-secondary)]"
      />
    </div>
  );
}
