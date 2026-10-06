#!/usr/bin/env node
// 엔소(円相, 붓으로 한 번에 그린 원) — 우로의 표지(개발 13). **모양의 정본은 이 파일 하나**이고, 나머지는 여기서 만든다:
//   src/lib/enso.ts            웹(부탁 점·제안 카드 머리)
//   ios/Shared/Enso.swift        iOS(ErrandMark)
//   src-tauri/icons/icon.svg   맥 앱 아이콘 원본 → npx tauri icon(아래)
//   src-tauri/icons/tray-*.svg 메뉴바 세 상태(대기 · 도는 중 · 답 도착) → 44px PNG
//   ios/icon.svg               iOS 앱 아이콘 원본 → 알파 없는 1024 PNG
// 쓰는 법: node scripts/enso.mjs   (rsvg-convert 필요: brew install librsvg. 맥 icns 는 따로 `npx tauri icon src-tauri/icons/icon.svg`)
//
// 모양: 24 단위 판, 중심 (12,12), 바깥 반지름 R. 바깥 가장자리는 참 원이고 굵기만 각도마다 바뀐다 —
// 왼쪽 아래가 굵고(붓을 누른 쪽) 오른쪽 위가 가늘며, 끝 80° 동안 붓끝처럼 0 으로 빠진다. 열린 틈(≈60°)은 위 오른쪽.
// 틈 = «아직 끝나지 않은 부탁», 닫히며 채워진 원 = «답이 와서 고리가 닫힘»(우로보로스). DESIGN.md «엔소».

import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const SHU = "#BF4329"; // 朱 — 라이트 액센트(App.css --accent)
const PAPER = "#FAFAF8"; // 미색 흰 — 라이트 바탕(App.css --canvas, 개발 16 에 키나리에서 바꿈)

const R = 9.5;
const A1 = -32; // 획이 시작하는 각(도, y 아래 방향 = 시계방향)
const A2 = 268; // 획이 끝나는 각
const STEP = 4;
const rad = (d) => (d * Math.PI) / 180;
const f = (n) => +n.toFixed(2);

/** 그 각에서의 획 굵기(24 판 단위). */
function width(d) {
  let w = 2.1 + 0.8 * Math.cos(rad(d - 140));
  const end = (A2 - d) / 80;
  if (end < 1) {
    const t = Math.max(end, 0);
    w *= t * t * (3 - 2 * t);
  }
  const start = (d - A1) / 10;
  if (start < 1) w *= 0.82 + 0.18 * start;
  return w;
}

const outer = [];
const inner = [];
for (let d = A1; d <= A2 + 1e-9; d += STEP) {
  const c = Math.cos(rad(d));
  const s = Math.sin(rad(d));
  outer.push([f(12 + R * c), f(12 + R * s)]);
  const ri = R - width(d);
  inner.push([f(12 + ri * c), f(12 + ri * s)]);
}
const POINTS = [...outer, ...inner.reverse()];
const PATH = "M" + POINTS.map(([x, y]) => `${x} ${y}`).join("L") + "Z";

// 도는 중 = 획의 앞 130° 만 짙게(«그리는 중»). 그 부분을 오려 낼 부채꼴.
const WEDGE = [[12, 12]];
for (let d = A1 - 8; d <= A1 + 130; d += 10) WEDGE.push([f(12 + 14 * Math.cos(rad(d))), f(12 + 14 * Math.sin(rad(d)))]);
const WEDGE_PATH = "M" + WEDGE.map(([x, y]) => `${x} ${y}`).join("L") + "Z";

const HEAD = "생성: scripts/enso.mjs — 손으로 고치지 말 것";
const write = (rel, text) => writeFileSync(join(ROOT, rel), text);

write(
  "src/lib/enso.ts",
  `// ${HEAD}.\n// 엔소(붓 한 번 원) — 24 단위 판. 열린 획, 도는 중에 짙게 칠할 부채꼴, 닫힌 원의 반지름.\n` +
    `export const ENSO_PATH = "${PATH}";\nexport const ENSO_RUN_CLIP = "${WEDGE_PATH}";\nexport const ENSO_R = ${R};\n`,
);

const swiftPts = (pts) => pts.map(([x, y]) => `(${x}, ${y})`).join(", ");
write(
  "ios/Shared/Enso.swift",
  `// ${HEAD}.\n// 엔소(붓 한 번 원) — 24 단위 판의 점들. 맥(src/lib/enso.ts)과 같은 모양.\n\nimport SwiftUI\n\n` +
    `enum EnsoGeometry {\n    static let r: CGFloat = ${R}\n    static let stroke: [(CGFloat, CGFloat)] = [${swiftPts(POINTS)}]\n` +
    `    static let runClip: [(CGFloat, CGFloat)] = [${swiftPts(WEDGE)}]\n}\n\n` +
    `/// 24 판의 점들을 받은 사각형에 맞춰 그린다.\nstruct EnsoShape: Shape {\n    var points: [(CGFloat, CGFloat)] = EnsoGeometry.stroke\n` +
    `    func path(in rect: CGRect) -> Path {\n        let k = min(rect.width, rect.height) / 24\n        var p = Path()\n` +
    `        for (i, (x, y)) in points.enumerated() {\n            let pt = CGPoint(x: rect.minX + x * k, y: rect.minY + y * k)\n` +
    `            if i == 0 { p.move(to: pt) } else { p.addLine(to: pt) }\n        }\n        p.closeSubpath()\n        return p\n    }\n}\n`,
);

const svg = (w, body, note) =>
  `<!-- ${note}\n     ${HEAD}. -->\n<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${w}" viewBox="0 0 ${w} ${w}">\n${body}\n</svg>\n`;
// 24 판을 반지름 rOut 의 원으로 놓는다.
const place = (cx, cy, rOut) => `translate(${f(cx - 12 * (rOut / R))} ${f(cy - 12 * (rOut / R))}) scale(${f(rOut / R)})`;

// 맥 앱 아이콘 — macOS 격자(824 판, 1024 캔버스). 미색 판에 주홍 엔소.
write(
  "src-tauri/icons/icon.svg",
  svg(
    1024,
    `  <rect x="100" y="100" width="824" height="824" rx="185" fill="${PAPER}"/>\n  <path transform="${place(512, 512, 268)}" d="${PATH}" fill="${SHU}"/>`,
    "맥 앱 아이콘 원본 — 미색 판 위 주홍 엔소. 수정 시 node scripts/enso.mjs → npx tauri icon src-tauri/icons/icon.svg",
  ),
);
// iOS — 모서리는 시스템이 깎으니 판을 꽉 채운다.
write(
  "ios/icon.svg",
  svg(
    1024,
    `  <rect width="1024" height="1024" fill="${PAPER}"/>\n  <path transform="${place(512, 512, 330)}" d="${PATH}" fill="${SHU}"/>`,
    "iOS 앱 아이콘 원본 — 맥과 같은 엔소, 판을 꽉 채운다(App Store 는 알파 없는 PNG).",
  ),
);
// 메뉴바 — 템플릿 이미지(검정 + 알파만, 시스템이 메뉴바 밝기에 맞춰 반전). 44 = 22pt @2x.
const TRAY = place(22, 22, 13);
const trays = {
  idle: `  <path transform="${TRAY}" d="${PATH}"/>`,
  running:
    `  <defs><clipPath id="w"><path d="${WEDGE_PATH}"/></clipPath></defs>\n` +
    `  <g transform="${TRAY}"><path d="${PATH}" opacity="0.35"/><path d="${PATH}" clip-path="url(#w)"/></g>`,
  done: `  <circle cx="22" cy="22" r="11"/>`,
};
const trayNote = {
  idle: "메뉴바 — 열린 엔소(부탁 없음·대기).",
  running: "메뉴바 — 도는 중: 획의 앞부분만 짙다(그리는 중).",
  done: "메뉴바 — 답 도착(안 읽음): 닫혀 채워진 원.",
};
for (const [k, body] of Object.entries(trays)) write(`src-tauri/icons/tray-${k}.svg`, svg(44, body, trayNote[k]));

const run = (cmd, args) => execFileSync(cmd, args, { cwd: ROOT, stdio: "inherit" });
for (const k of Object.keys(trays)) run("rsvg-convert", ["-w", "44", "-h", "44", `src-tauri/icons/tray-${k}.svg`, "-o", `src-tauri/icons/tray-${k}.png`]);
// iOS 1024 — 알파를 빼려고 BMP 를 한 번 거친다(무손실).
const tmp = join(ROOT, "ios/icon-tmp.png");
run("rsvg-convert", ["-w", "1024", "-h", "1024", "ios/icon.svg", "-o", tmp]);
run("sips", ["-s", "format", "bmp", tmp, "--out", tmp + ".bmp"]);
run("sips", ["-s", "format", "png", tmp + ".bmp", "--out", "ios/Ouro/Assets.xcassets/AppIcon.appiconset/icon-1024.png"]);
run("rm", [tmp, tmp + ".bmp"]);
console.log("엔소: enso.ts · Enso.swift · 아이콘 원본 · 메뉴바 PNG 3 · iOS 1024 를 만들었어요. 맥 icns 는 npx tauri icon src-tauri/icons/icon.svg");
