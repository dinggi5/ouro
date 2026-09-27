// 앱 루트 — 개발 1 은 빈 셸이다: 오늘 날짜 + 빈 상태 한 문장.
// 일정 목록·달력은 개발 2(PLAN §12)에서 이 자리를 채운다.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

const dateFmt = new Intl.DateTimeFormat("ko-KR", { month: "long", day: "numeric" });
const weekdayFmt = new Intl.DateTimeFormat("ko-KR", { weekday: "long" });

/** 다음 자정까지 남은 ms. 팝오버는 며칠씩 떠 있을 수 있어 날짜가 스스로 넘어가야 한다. */
function msUntilMidnight(now: Date): number {
  const next = new Date(now);
  next.setHours(24, 0, 0, 0);
  return next.getTime() - now.getTime();
}

/** 오늘 날짜. 자정에 넘어가고, 창이 다시 보일 때도 다시 읽는다 — 맥이 잠든 사이 자정 타이머는
 *  늦게 깨므로 타이머 하나만 믿지 않는다. */
function useToday(): Date {
  const [today, setToday] = useState(() => new Date());
  useEffect(() => {
    let timer: number;
    const tick = () => {
      const now = new Date();
      setToday(now);
      timer = window.setTimeout(tick, msUntilMidnight(now) + 50);
    };
    timer = window.setTimeout(tick, msUntilMidnight(new Date()) + 50);
    const onVisible = () => {
      if (document.visibilityState === "visible") setToday(new Date());
    };
    document.addEventListener("visibilitychange", onVisible);
    window.addEventListener("focus", onVisible);
    return () => {
      window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisible);
      window.removeEventListener("focus", onVisible);
    };
  }, []);
  return today;
}

function App() {
  const today = useToday();

  // ⌘W = 팝오버 닫기. 창이 테두리 없음(decorations: false)이라 AppKit 이 앱 메뉴의 「닫기」를
  // 꺼 두어 러스트의 CloseRequested 까지 오지 않는다(Kura 개발 58 실측). 웹뷰에서 직접 받는다.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
      if (e.key.toLowerCase() !== "w") return;
      e.preventDefault();
      void invoke("hide_popover");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <main className="flex h-screen w-full flex-col overflow-y-auto rounded-lg bg-canvas px-5 pt-6 pb-5 text-ink">
      <header>
        <h1 className="num text-subheading font-semibold">{dateFmt.format(today)}</h1>
        <p className="mt-1 text-body-sm text-ink-muted">{weekdayFmt.format(today)}</p>
      </header>

      <div className="flex flex-1 items-center justify-center">
        <p className="text-body-sm text-ink-muted">오늘은 비어 있어요</p>
      </div>
    </main>
  );
}

export default App;
