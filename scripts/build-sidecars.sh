#!/usr/bin/env bash
# ouro-mcp 를 Tauri 사이드카(externalBin) 자리에 놓는다 (개발 4, 개발 8 에 Kura `scripts/build-sidecars.sh` 의 유니버설·STRICT 를 붙임).
#
#   ./scripts/build-sidecars.sh                                         호스트 아키텍처만
#   ./scripts/build-sidecars.sh aarch64-apple-darwin x86_64-apple-darwin  유니버설 빌드용
#
# Tauri v2 규칙: 설정의 `binaries/ouro-mcp` 는 파일이 `binaries/ouro-mcp-<타깃트리플>` 이어야 하고,
# 번들에선 트리플이 떨어져 `Ouro.app/Contents/MacOS/ouro-mcp` 로 놓인다 — MCP 등록은 그 절대경로 한 줄.
set -euo pipefail
cd "$(dirname "$0")/.."

die() { printf '\033[31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

OUT_DIR="src-tauri/binaries"
HOST="$(rustc -vV | awk '/^host:/{print $2}')"
[[ -n "$HOST" ]] || die "rustc -vV 에 host 줄이 없다"
TARGETS=("$@")
[[ ${#TARGETS[@]} -gt 0 ]] || TARGETS=("$HOST")

# 소스보다 산출물이 새것이면 cargo 를 안 부른다(tauri build 의 beforeBuildCommand 로도 불린다).
# 스크립트 자신도 소스다 — 빌드 방식을 바꿨는데 옛 산출물이 «최신» 으로 통과하지 않게.
needs_build() {
  local t out
  for t in "${TARGETS[@]}"; do
    out="$OUT_DIR/ouro-mcp-$t"
    [[ -f "$out" ]] || return 0
    [[ -z "$(find ouro-mcp/src ouro-mcp/Cargo.toml ouro-mcp/Cargo.lock scripts/build-sidecars.sh -newer "$out" -print -quit)" ]] || return 0
  done
  return 1
}

# STRICT(= release.sh) 에서는 만들지 않는다. 여기서 만들어야 하는 상황 자체가 «자격증명이 실린 빌드 안에서 cargo 가 돈다» 는 뜻이라
# 멈추는 게 맞다 — release.sh 는 자격증명을 싣기 **전에** 이 스크립트를 직접 부른다(Kura 개발 30·44).
if [[ "${OURO_SIDECARS_STRICT:-0}" == "1" ]]; then
  needs_build && die "사이드카가 최신이 아닌데 STRICT 모드다 — release.sh 가 빌드 전에 부르는 곳에서 실패했을 가능성이 크다"
  echo "  ✓ 사이드카 최신 (STRICT: 빌드 안 함)"
  exit 0
fi
if ! needs_build; then
  echo "  ✓ 사이드카 최신 (${TARGETS[*]})"
  exit 0
fi

mkdir -p "$OUT_DIR"
for t in "${TARGETS[@]}"; do
  # 호스트와 같은 타깃이면 --target 을 안 붙인다 — 붙이면 target/<트리플>/ 아래에 의존성 트리를 한 벌 더 만든다(Kura 실측: 수 GB).
  TARGET_ARGS=()
  REL_DIR="ouro-mcp/target/release"
  if [[ "$t" != "$HOST" ]]; then
    rustup target list --installed 2>/dev/null | grep -qx "$t" || die "타깃 $t 가 설치돼 있지 않다:  rustup target add $t"
    TARGET_ARGS=(--target "$t")
    REL_DIR="ouro-mcp/target/$t/release"
  fi
  # --locked: 배포본에 들어가는 바이너리라 빌드가 의존성 버전을 조용히 올리지 않게.
  cargo build --release --locked --manifest-path ouro-mcp/Cargo.toml "${TARGET_ARGS[@]+"${TARGET_ARGS[@]}"}"
  OUT="$OUT_DIR/ouro-mcp-$t"
  # 덮어쓰지 않고 지우고 복사한다 — 떠 있는 MCP 서버의 실행파일을 덮어쓰면 그 프로세스가 조용히 죽는다(Kura 실측).
  rm -f "$OUT"
  cp "$REL_DIR/ouro-mcp" "$OUT"
  chmod +x "$OUT"
  echo "  ✓ $(basename "$OUT")  $(du -h "$OUT" | cut -f1)"
done
