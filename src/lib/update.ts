// 인앱 업데이트 — 러스트 `update.rs` 의 두 문(check_update · install_update)만 쓴다. 플러그인을 직접 부르지 않는다
// (capabilities 에 updater 권한이 없다 — 설치 전 가드를 웹뷰가 건너뛰지 못하게).
//
// 저절로 하는 건 «확인» 뿐: 켤 때 한 번(30초 뒤 — 켜자마자 네트워크를 붙잡지 않게)과 12시간마다. 저절로 확인하다 난 오류는 조용히 넘긴다
// (오프라인은 흔하다). 사람이 메뉴바 «업데이트 확인…» 을 눌렀을 때만 «최신이에요»·오류를 보인다.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { errorText } from "./events";

export type UpdateInfo = { version: string; currentVersion: string; notes: string | null };
type Progress = { downloaded: number; total: number | null };

const FIRST_CHECK_MS = 30_000;
const EVERY_MS = 12 * 60 * 60 * 1000;

export type UpdateState = {
  info: UpdateInfo | null;
  /** 카드를 펼쳤나(노트·설치 버튼). 저절로 찾았을 땐 아래 한 줄만, 누르면 펼친다. */
  open: boolean;
  /** 사람이 확인을 눌렀는데 최신이었을 때의 한 줄 / 오류 */
  message: string | null;
  checking: boolean;
  installing: boolean;
  /** 0~1, 크기를 모르면 null */
  progress: number | null;
  show: () => void;
  close: () => void;
  install: () => void;
};

export function useUpdate(): UpdateState {
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [open, setOpen] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<number | null>(null);
  const busy = useRef(false);

  const check = useCallback(async (manual: boolean) => {
    if (busy.current) return;
    busy.current = true;
    setChecking(true);
    if (manual) setMessage(null);
    try {
      const found = await invoke<UpdateInfo | null>("check_update");
      setInfo(found);
      if (manual) {
        setOpen(true);
        if (!found) setMessage("최신 버전이에요");
      }
    } catch (e) {
      if (manual) {
        setOpen(true);
        setMessage(errorText(e));
      }
    } finally {
      busy.current = false;
      setChecking(false);
    }
  }, []);

  useEffect(() => {
    const first = window.setTimeout(() => void check(false), FIRST_CHECK_MS);
    const every = window.setInterval(() => void check(false), EVERY_MS);
    let unlisten: (() => void)[] = [];
    let alive = true;
    const keep = (u: () => void) => (alive ? unlisten.push(u) : u());
    listen("update-check", () => void check(true)).then(keep, () => {});
    listen<Progress>("update-progress", (e) => {
      const { downloaded, total } = e.payload;
      setProgress(total ? Math.min(1, downloaded / total) : null);
    }).then(keep, () => {});
    return () => {
      alive = false;
      window.clearTimeout(first);
      window.clearInterval(every);
      unlisten.forEach((u) => u());
      unlisten = [];
    };
  }, [check]);

  const install = useCallback(() => {
    if (installing) return;
    setInstalling(true);
    setMessage(null);
    setProgress(null);
    // 성공하면 앱이 다시 켜져 여기로 돌아오지 않는다. 돌아왔다면 실패(부탁이 도는 중 등) — 문장을 보인다.
    invoke("install_update").catch((e) => {
      setInstalling(false);
      setMessage(errorText(e));
    });
  }, [installing]);

  return {
    info,
    open,
    message,
    checking,
    installing,
    progress,
    show: () => setOpen(true),
    close: () => {
      if (installing) return;
      setOpen(false);
      setMessage(null);
    },
    install,
  };
}
