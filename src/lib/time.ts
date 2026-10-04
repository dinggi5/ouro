// 날짜 도우미 — 전부 **로컬 시간대**의 달력 날짜로 센다. 러스트 쪽(store.rs 의 date_window)도 같은 시간대를 쓴다.
// 하루 더하기는 `setDate` 로 한다 — 24시간을 밀리초로 더하면 서머타임 날에 하루가 23·25시간이라 어긋난다.

export function startOfDay(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate());
}

export function addDays(d: Date, n: number): Date {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);
}

/** 한 주는 일요일부터(한국 달력의 관례). */
export function startOfWeek(d: Date): Date {
  return addDays(startOfDay(d), -d.getDay());
}

export function startOfMonth(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), 1);
}

/** 달을 옮긴다. 31일에서 한 달 뒤가 다다음 달 1일로 넘치지 않게 그 달 마지막 날로 자른다. */
export function addMonths(d: Date, n: number): Date {
  const first = new Date(d.getFullYear(), d.getMonth() + n, 1);
  const last = new Date(first.getFullYear(), first.getMonth() + 1, 0).getDate();
  return new Date(first.getFullYear(), first.getMonth(), Math.min(d.getDate(), last));
}

export function sameDay(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
}

const pad = (n: number) => String(n).padStart(2, "0");

/** 로컬 날짜 → `YYYY-MM-DD` (종일 일정의 저장 모양, `<input type="date">` 의 값). */
export function ymd(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

export function parseYmd(s: string): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(s);
  if (!m) return null;
  const d = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
  return ymd(d) === s ? d : null; // 2월 30일 같은 건 거른다
}

/** 로컬 시각 → `HH:MM` (`<input type="time">` 의 값). */
export function hm(d: Date): string {
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** 날짜 `YYYY-MM-DD` + 시각 `HH:MM` → 로컬 Date.
 *  서머타임이 시작되는 날의 없는 시각(예: 뉴욕 3월 둘째 일요일 02:30)은 JS 가 조용히 03:30 으로 밀어 버린다 —
 *  사람이 본 시각과 저장된 시각이 달라지므로 null 로 돌려 칸을 고치게 한다(코덱스 개발 2). */
export function combine(date: string, time: string): Date | null {
  const d = parseYmd(date);
  const t = /^(\d{2}):(\d{2})$/.exec(time);
  if (!d || !t) return null;
  const [h, m] = [Number(t[1]), Number(t[2])];
  d.setHours(h, m, 0, 0);
  if (d.getHours() !== h || d.getMinutes() !== m || ymd(d) !== date) return null;
  return d;
}

/** 다음 정각. 새 일정의 기본 시작 시각. */
export function nextHour(now: Date): Date {
  const d = new Date(now);
  d.setHours(d.getHours() + 1, 0, 0, 0);
  return d;
}

const makeFmt = () => ({
  monthDay: new Intl.DateTimeFormat("ko-KR", { month: "long", day: "numeric" }),
  weekday: new Intl.DateTimeFormat("ko-KR", { weekday: "long" }),
  weekdayShort: new Intl.DateTimeFormat("ko-KR", { weekday: "short" }),
  yearMonth: new Intl.DateTimeFormat("ko-KR", { year: "numeric", month: "long" }),
  time: new Intl.DateTimeFormat("ko-KR", { hour: "numeric", minute: "2-digit" }),
});
type Fmt = ReturnType<typeof makeFmt>;

// 포매터는 만들 때의 시간대에 묶인다 — 팝오버가 며칠 떠 있는 동안 시간대를 바꾸면(여행) `Date` 는 새 시간대인데 글자는 옛 시간대가 된다
// (코덱스 개발 5 P2). 1초에 한 번만 시간대를 물어 바뀌었으면 다시 만든다(줄마다 부르는 것이라 매번 묻지 않는다).
const zoneNow = () => new Intl.DateTimeFormat().resolvedOptions().timeZone;
let made: Fmt = makeFmt();
let madeZone = zoneNow();
let checkedAt = 0;
function current(): Fmt {
  const t = Date.now();
  if (t - checkedAt > 1000) {
    checkedAt = t;
    const z = zoneNow();
    if (z !== madeZone) {
      made = makeFmt();
      madeZone = z;
    }
  }
  return made;
}

export const fmt: Fmt = {
  get monthDay() {
    return current().monthDay;
  },
  get weekday() {
    return current().weekday;
  },
  get weekdayShort() {
    return current().weekdayShort;
  },
  get yearMonth() {
    return current().yearMonth;
  },
  get time() {
    return current().time;
  },
};

/** 다음 자정까지 남은 ms. 팝오버는 며칠씩 떠 있을 수 있어 날짜가 스스로 넘어가야 한다. */
export function msUntilMidnight(now: Date): number {
  return addDays(startOfDay(now), 1).getTime() - now.getTime();
}
