// 설정 창 (개발 16) — 창 이름 `settings`(settings.rs). 카드 셋: iCloud 동기화 · 아침 브리핑 · 업데이트.
// 그전엔 메뉴바 메뉴와 팝오버 카드에 흩어져 있던 것들이다. 설명 문구는 두지 않는다 — 줄 이름과 값만(사장 지시: «깔끔 미니멀»).
// 앱의 일이라 朱 없음(DESIGN «색은 AI 몫»).

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";
import { syncStateLine } from "./components/SyncCard";
import { errandApi, minutesLabel } from "./lib/errands";
import { errorText } from "./lib/events";
import { useSync } from "./lib/sync";
import { appVersion, useUpdate } from "./lib/update";

/** 00:00~23:30, 30분 단위(러스트 `brief::valid_at`). */
const TIMES = Array.from({ length: 48 }, (_, i) => i * 30);

export default function Settings() {
  const sync = useSync();
  const update = useUpdate(false);
  const [version, setVersion] = useState("");
  const [morning, setMorning] = useState<{ on: boolean; at: number } | null>(null);
  const [autoUpdate, setAutoUpdate] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());

  const loadBrief = useCallback(() => {
    errandApi.briefing().then((b) => setMorning({ on: b.morning, at: b.morningAt }), () => {});
  }, []);

  useEffect(() => {
    void appVersion().then(setVersion);
    invoke<boolean>("update_auto").then(setAutoUpdate, () => {});
    loadBrief();
    // 열 때 한 번 조용히 — 팝오버가 이미 찾은 새 버전이 여기서도 보이게(창마다 상태가 따로다). 자동 확인을 껐으면 안 묻는다.
    update.checkQuietly();
    const tick = window.setInterval(() => setNow(Date.now()), 30_000);
    const offs: Array<() => void> = [];
    let dead = false;
    const keep = (f: () => void) => (dead ? f() : offs.push(f));
    // 팝오버의 «아침 알림» 을 눌렀으면 여기도 맞춘다(lib.rs `touched`).
    listen("data-changed", loadBrief).then(keep, () => {});
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey && e.key.toLowerCase() === "w") {
        e.preventDefault();
        void invoke("close_window");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      dead = true;
      offs.forEach((f) => f());
      window.clearInterval(tick);
      window.removeEventListener("keydown", onKey);
    };
    // 열 때 한 번만.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const save = async (f: () => Promise<void>) => {
    setError(null);
    try {
      await f();
    } catch (e) {
      setError(errorText(e));
    }
  };

  const s = sync.status;
  const pct = update.progress === null ? null : Math.round(update.progress * 100);
  const updateSub = update.checking ? "확인하는 중…" : update.info ? undefined : (update.message ?? undefined);

  return (
    <main className="flex h-screen flex-col gap-4 overflow-y-auto bg-canvas px-6 pt-6 pb-8 text-ink">
      {s?.available && (
        <Card>
          <Row
            label="iCloud 동기화"
            sub={s.on ? syncStateLine(s, now) : undefined}
            strong={s.on && !!s.error}
            control={<Switch on={s.on} disabled={sync.busy} label="iCloud 동기화" onChange={sync.toggle} />}
          />
          {s.on && (
            <Row
              label="실행 맥"
              sub={s.runnerIsMe ? "이 맥" : (s.runnerName ?? "없음")}
              control={
                !s.runnerIsMe && (
                  <TextButton disabled={sync.busy} onClick={sync.makeRunner}>
                    이 맥으로
                  </TextButton>
                )
              }
            />
          )}
        </Card>
      )}

      {morning && (
        <Card>
          <Row
            label="아침 브리핑"
            control={
              <Switch
                on={morning.on}
                label="아침 브리핑"
                onChange={() =>
                  void save(async () => {
                    await errandApi.setMorning(!morning.on);
                    setMorning({ ...morning, on: !morning.on });
                  })
                }
              />
            }
          />
          {morning.on && (
            <Row
              label="시각"
              control={
                <select
                  aria-label="아침 브리핑 시각"
                  value={morning.at}
                  onChange={(e) => {
                    const at = Number(e.target.value);
                    void save(async () => {
                      await errandApi.setMorningAt(at);
                      setMorning({ ...morning, at });
                    });
                  }}
                  className="num h-8 rounded-pill bg-surface-sunken px-3 text-label text-ink outline-none"
                >
                  {TIMES.map((m) => (
                    <option key={m} value={m}>
                      {minutesLabel(m)}
                    </option>
                  ))}
                </select>
              }
            />
          )}
        </Card>
      )}

      <Card>
        {autoUpdate !== null && (
          <Row
            label="자동으로 업데이트 확인"
            control={
              <Switch
                on={autoUpdate}
                label="자동으로 업데이트 확인"
                onChange={() =>
                  void save(async () => {
                    await invoke("set_update_auto", { on: !autoUpdate });
                    setAutoUpdate(!autoUpdate);
                  })
                }
              />
            }
          />
        )}
        <Row
          label={<span className="num">Ouro {version}</span>}
          sub={updateSub}
          control={
            !update.info && (
              <TextButton disabled={update.checking || update.installing} onClick={update.check}>
                확인
              </TextButton>
            )
          }
        />
        {update.info && (
          <div className="flex flex-col gap-2 px-4 pt-3 pb-4">
            <p className="num text-body-sm font-semibold">새 버전 {update.info.version}</p>
            {update.info.notes && (
              <p className="select-text max-h-48 overflow-y-auto whitespace-pre-wrap break-words rounded-md bg-surface-sunken px-3 py-2.5 text-caption">
                {update.info.notes}
              </p>
            )}
            {update.message && (
              <p role="alert" className="text-micro">
                {update.message}
              </p>
            )}
            <button
              type="button"
              disabled={update.installing}
              onClick={update.install}
              className="num mt-1 h-10 rounded-md bg-ink text-label font-semibold text-canvas transition-opacity duration-100 active:opacity-80 disabled:opacity-40"
            >
              {update.installing ? (pct === null ? "내려받는 중…" : `내려받는 중 ${pct}%`) : "설치하고 다시 켜기"}
            </button>
          </div>
        )}
      </Card>

      {(error ?? sync.message) && (
        <p role="alert" className="px-1 text-micro text-ink">
          {error ?? sync.message}
        </p>
      )}
    </main>
  );
}

/** 줄들을 한 장의 면에 — 줄 사이는 머리카락 선. */
function Card({ children }: { children: React.ReactNode }) {
  return (
    <section className="shrink-0 overflow-hidden rounded-lg bg-surface ring-1 ring-hairline [&>*+*]:border-t [&>*+*]:border-hairline">
      {children}
    </section>
  );
}

function Row({
  label,
  sub,
  control,
  strong,
}: {
  label: React.ReactNode;
  sub?: string;
  control?: React.ReactNode;
  /** 아랫줄이 문제를 말한다 — 흐리게 두지 않는다(빨강 없음, 먹색으로). */
  strong?: boolean;
}) {
  return (
    <div className="flex min-h-13 items-center justify-between gap-4 px-4 py-3">
      <div className="min-w-0">
        <p className="whitespace-nowrap text-body-sm">{label}</p>
        {sub && (
          <p role="status" className={`num mt-0.5 truncate text-caption ${strong ? "text-ink" : "text-ink-muted"}`}>
            {sub}
          </p>
        )}
      </div>
      {control && <div className="shrink-0">{control}</div>}
    </div>
  );
}

function Switch({ on, disabled, label, onChange }: { on: boolean; disabled?: boolean; label: string; onChange: () => void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      disabled={disabled}
      onClick={onChange}
      className={`relative block h-6 w-10 rounded-pill transition-colors duration-200 ease-out disabled:opacity-40 ${on ? "bg-ink" : "bg-hairline"}`}
    >
      <span
        className={`absolute top-0.5 left-0.5 h-5 w-5 rounded-pill bg-surface shadow-sm transition-transform duration-200 ease-out ${on ? "translate-x-4" : ""}`}
      />
    </button>
  );
}

function TextButton({ disabled, onClick, children }: { disabled?: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className="h-8 rounded-pill bg-surface-sunken px-3.5 text-label font-semibold text-ink transition-opacity duration-100 active:opacity-80 disabled:opacity-40"
    >
      {children}
    </button>
  );
}
