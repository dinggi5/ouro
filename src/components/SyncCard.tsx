// iCloud 동기화 — 팝오버 아래 한 줄. 켜고 끄기·실행 맥은 설정 창(Settings.tsx, 개발 16)으로 옮겼다.
// 평소엔 아무것도 안 보인다 — 켜져 있는데 문제가 있을 때만 한 줄, 누르면 설정 창.

import { invoke } from "@tauri-apps/api/core";
import { agoText, type SyncState, type SyncStatus } from "../lib/sync";

/** 지금 상태 한 줄 — 설정 창의 동기화 칸. */
export function syncStateLine(s: SyncStatus, now: number): string {
  if (!s.on) return "꺼져 있어요 — 일정은 이 맥에만 있어요";
  if (s.error) return s.error;
  if (s.pending > 0) return `보내는 중 · ${s.pending}개 남음`;
  if (s.lastSync) return `맞췄어요 · ${agoText(s.lastSync, now)}`;
  return "처음 맞추는 중…";
}

/** 켜져 있는데 문제가 있을 때만 아래 한 줄 — 누르면 설정 창. */
export function SyncLine({ sync }: { sync: SyncState }) {
  const s = sync.status;
  if (!s?.on || !s.error) return null;
  return (
    <button
      type="button"
      onClick={() => void invoke("open_settings")}
      className="flex shrink-0 items-center px-5 pt-2 text-left text-micro text-ink"
    >
      {s.error} · 설정
    </button>
  );
}
