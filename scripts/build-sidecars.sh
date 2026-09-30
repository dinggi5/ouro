#!/usr/bin/env bash
# ouro-mcp 를 Tauri 사이드카(externalBin) 자리에 놓는다 (개발 4, Kura `scripts/build-sidecars.sh` 에서 줄여 옴).
#
#   ./scripts/build-sidecars.sh          호스트 아키텍처만
#
# Tauri v2 규칙: 설정의 `binaries/ouro-mcp` 는 파일이 `binaries/ouro-mcp-<타깃트리플>` 이어야 하고,
# 번들에선 트리플이 떨어져 `Ouro.app/Contents/MacOS/ouro-mcp` 로 놓인다 — MCP 등록은 그 절대경로 한 줄.
# 유니버설 빌드·서명 중 자격증명 격리(Kura 의 STRICT 모드)는 배포(개발 8)에서 붙인다.
set -euo pipefail
cd "$(dirname "$0")/.."

OUT_DIR="src-tauri/binaries"
HOST="$(rustc -vV | awk '/^host:/{print $2}')"
[[ -n "$HOST" ]] || { echo "rustc -vV 에 host 줄이 없다" >&2; exit 1; }
OUT="$OUT_DIR/ouro-mcp-$HOST"

# 소스보다 산출물이 새것이면 cargo 를 안 부른다(tauri build 의 beforeBuildCommand 로도 불린다).
# 스크립트 자신도 소스다 — 빌드 방식을 바꿨는데 옛 산출물이 «최신» 으로 통과하지 않게.
if [[ -f "$OUT" ]] && [[ -z "$(find ouro-mcp/src ouro-mcp/Cargo.toml ouro-mcp/Cargo.lock scripts/build-sidecars.sh -newer "$OUT" -print -quit)" ]]; then
  echo "  ✓ 사이드카 최신"
  exit 0
fi

# --locked: 배포본에 들어가는 바이너리라 빌드가 의존성 버전을 조용히 올리지 않게.
cargo build --release --locked --manifest-path ouro-mcp/Cargo.toml
mkdir -p "$OUT_DIR"
# 덮어쓰지 않고 지우고 복사한다 — 떠 있는 MCP 서버의 실행파일을 덮어쓰면 그 프로세스가 조용히 죽는다(Kura 실측).
rm -f "$OUT"
cp ouro-mcp/target/release/ouro-mcp "$OUT"
chmod +x "$OUT"
echo "  ✓ $(basename "$OUT")  $(du -h "$OUT" | cut -f1)"
