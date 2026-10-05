// iCloud 동기화 — 러스트 `sync.rs` 의 네 문(sync_status · set_sync · make_runner · sync_fetch)만 쓴다.
// 카드는 메뉴바 «iCloud 동기화…» 로만 연다(헬퍼가 든 앱에만 그 메뉴가 있다). 평소엔 아무것도 안 보인다 — 켜져 있는데 문제가 있을 때만 아래 한 줄.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { errorText } from "./events";

export type SyncStatus = {
  available: boolean;
  on: boolean;
  account: string | null;
  lastSync: number | null;
  error: string | null;
  pending: number;
  runnerName: string | null;
  runnerIsMe: boolean;
  thisName: string;
};

export type SyncState = {
  status: SyncStatus | null;
  open: boolean;
  busy: boolean;
  /** 누른 일이 실패했을 때의 한 줄(상태의 error 와 따로 — 상태 오류는 헬퍼가 알려 준 것) */
  message: string | null;
  show: () => void;
  close: () => void;
  toggle: () => void;
  makeRunner: () => void;
};

export function useSync(): SyncState {
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const load = useCallback(() => {
    invoke<SyncStatus>("sync_status").then(setStatus, () => {});
  }, []);

  useEffect(() => {
    load();
    const offs: Array<() => void> = [];
    let dead = false;
    const keep = (f: () => void) => (dead ? f() : offs.push(f));
    listen("sync-changed", load).then(keep, () => {});
    listen("sync-open", () => {
      setMessage(null);
      setOpen(true);
      load();
    }).then(keep, () => {});
    // 팝오버가 보일 때마다 다른 기기의 변경을 바로 받는다(푸시를 놓쳤을 때의 보험). 꺼져 있으면 러스트가 그냥 넘긴다.
    const onVisible = () => {
      if (document.visibilityState === "visible") invoke("sync_fetch").catch(() => {});
    };
    window.addEventListener("focus", onVisible);
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      dead = true;
      offs.forEach((f) => f());
      window.removeEventListener("focus", onVisible);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [load]);

  const run = useCallback(async (f: () => Promise<SyncStatus>) => {
    setBusy(true);
    setMessage(null);
    try {
      setStatus(await f());
    } catch (e) {
      setMessage(errorText(e));
    } finally {
      setBusy(false);
    }
  }, []);

  return {
    status,
    open,
    busy,
    message,
    show: () => setOpen(true),
    close: () => setOpen(false),
    toggle: () => void run(() => invoke<SyncStatus>("set_sync", { on: !status?.on })),
    makeRunner: () => void run(() => invoke<SyncStatus>("make_runner")),
  };
}

/** «방금» · «3분 전» · «오후 2:14» */
export function agoText(ms: number, now: number): string {
  const d = Math.max(0, now - ms);
  if (d < 60_000) return "방금";
  if (d < 60 * 60_000) return `${Math.floor(d / 60_000)}분 전`;
  return new Date(ms).toLocaleTimeString("ko-KR", { hour: "numeric", minute: "2-digit" });
}
