// 인앱 업데이트 — 러스트 `update.rs` 의 두 문(check_update · install_update)만 쓴다. 플러그인을 직접 부르지 않는다
// (capabilities 에 updater 권한이 없다 — 설치 전 가드를 웹뷰가 건너뛰지 못하게).
//
// 저절로 하는 건 «확인» 뿐: 켤 때 한 번(30초 뒤 — 켜자마자 네트워크를 붙잡지 않게)과 그 뒤 주기적으로 — 팝오버 창만(`auto`),
// 설정 창의 «자동으로 확인» 이 켜져 있을 때만(기본 켬, 개발 16 — 러스트 `update_auto`).
// 저절로 확인하다 난 오류는 조용히 넘긴다(오프라인은 흔하다). 사람이 설정 창의 «확인» 을 눌렀을 때만 «최신이에요»·오류를 보인다(개발 16).

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
  /** 사람이 누른 확인 — 결과(최신·오류)를 `message` 로. */
  check: () => void;
  /** 조용한 확인 — 새 버전이 있을 때만 `info` 가 찬다(설정 창을 열 때). */
  checkQuietly: () => void;
};

/** 지금 버전 — «최신 버전이에요 · 1.0.0». 바뀌지 않으니 한 번만 묻는다. */
let versionAsk: Promise<string> | null = null;
export function appVersion(): Promise<string> {
  versionAsk ??= invoke<{ version: string }>("app_info").then(
    (i) => i.version,
    () => {
      versionAsk = null;
      return "";
    },
  );
  return versionAsk;
}

export function latestText(version: string): string {
  return version ? `최신 버전이에요 · ${version}` : "최신 버전이에요";
}

/** `auto` = 켤 때·12시간마다 저절로 확인한다(팝오버만 — 크게 보기·설정 창까지 돌면 같은 확인이 겹친다). */
export function useUpdate(auto = true): UpdateState {
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [open, setOpen] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<number | null>(null);
  const busy = useRef(false);
  // 설치 중엔 확인하지 않는다 — 받는 동안 새 버전이 화면을 덮으면 «본 것과 다른 것» 을 누르게 된다(러스트도 막는다).
  const installingRef = useRef(false);

  // 설정의 «자동으로 확인». 모르는 동안은 켬으로 본다(기본값).
  const autoOn = useRef(true);

  const check = useCallback(async (manual: boolean) => {
    if (busy.current || installingRef.current) return;
    if (!manual && !autoOn.current) return;
    busy.current = true;
    setChecking(true);
    if (manual) setMessage(null);
    try {
      const found = await invoke<UpdateInfo | null>("check_update");
      setInfo(found);
      if (manual) {
        setOpen(true);
        if (!found) setMessage(latestText(await appVersion()));
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
    const first = auto ? window.setTimeout(() => void check(false), FIRST_CHECK_MS) : undefined;
    const every = auto ? window.setInterval(() => void check(false), EVERY_MS) : undefined;
    let unlisten: (() => void)[] = [];
    let alive = true;
    const keep = (u: () => void) => (alive ? unlisten.push(u) : u());
    invoke<boolean>("update_auto").then((on) => (autoOn.current = on), () => {});
    listen<boolean>("update-auto-changed", (e) => (autoOn.current = e.payload)).then(keep, () => {});
    // 다른 창에서 설치를 시작·실패했다(개발 12) — 이 창도 같이 «설치 중» 이 돼 새 초안을 막는다.
    listen<boolean>("update-installing", (e) => {
      installingRef.current = e.payload;
      setInstalling(e.payload);
      if (e.payload) setProgress(null);
    }).then(keep, () => {});
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
  }, [check, auto]);

  const install = useCallback(() => {
    if (installingRef.current) return;
    installingRef.current = true;
    setInstalling(true);
    setMessage(null);
    setProgress(null);
    // 성공하면 앱이 다시 켜져 여기로 돌아오지 않는다. 돌아왔다면 실패(부탁이 도는 중 등) — 문장을 보인다.
    // 본 버전을 같이 보낸다 — 다른 창의 확인이 후보를 바꿨으면 러스트가 거절한다.
    invoke("install_update", { version: info?.version ?? "" }).catch((e) => {
      installingRef.current = false;
      setInstalling(false);
      setMessage(errorText(e));
    });
  }, [info]);

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
    check: () => void check(true),
    checkQuietly: () => void check(false),
  };
}
