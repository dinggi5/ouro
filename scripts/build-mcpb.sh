#!/usr/bin/env bash
# Claude 데스크톱 확장(.mcpb) 만들기 (개발 8, Kura `scripts/build-mcpb.sh` 에서 줄여 옴).
#
#   ./scripts/build-mcpb.sh              → src-tauri/target/mcpb/ouro-<버전>.mcpb
#
# 확장에는 **실행 파일이 없다**. manifest·아이콘·런처 셸(mcpb/server/ouro-mcp)뿐이고, 런처는 설치된 Ouro.app 안의
# 서명·공증된 ouro-mcp 를 서명 확인 뒤 exec 한다. 그래서 이 산출물은 서명·공증 대상이 아니다 — 아래 «맥오 금지» 가 그 전제를 지킨다.
# Kura 와 달리 앱 Resources 에 동봉하지 않는다(우로우로엔 «Claude 데스크톱에 연결» 버튼이 없다) — 릴리스 자산으로만 나간다.
set -euo pipefail
cd "$(dirname "$0")/.."

ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
die()  { printf '\033[31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

SRC_DIR="mcpb"
OUT_DIR="src-tauri/target/mcpb"
STAGE="$OUT_DIR/stage"

command -v npx >/dev/null || die "npx 가 없다 (Node 필요)"
[[ -f "$SRC_DIR/manifest.json" ]] || die "$SRC_DIR/manifest.json 이 없다"
[[ -x "$SRC_DIR/server/ouro-mcp" ]] || die "$SRC_DIR/server/ouro-mcp 에 실행 비트가 없다:  chmod +x $SRC_DIR/server/ouro-mcp"

VERSION_CONF="$(python3 -c 'import json;print(json.load(open("src-tauri/tauri.conf.json"))["version"])')"
VERSION_MCPB="$(python3 -c 'import json;print(json.load(open("mcpb/manifest.json"))["version"])')"
# 갈리면 사용자가 받은 확장이 어느 앱 버전을 위한 건지 알 수 없다. release.sh 사전 점검에도 같은 검사가 있다.
[[ "$VERSION_CONF" == "$VERSION_MCPB" ]] || die "버전 불일치 — tauri.conf.json=$VERSION_CONF / mcpb/manifest.json=$VERSION_MCPB"

MCPB_FILE="$OUT_DIR/ouro-$VERSION_CONF.mcpb"
rm -rf "$STAGE"
mkdir -p "$STAGE/server"
cp "$SRC_DIR/manifest.json" "$STAGE/manifest.json"
cp "$SRC_DIR/server/ouro-mcp" "$STAGE/server/ouro-mcp"
chmod +x "$STAGE/server/ouro-mcp"
# 아이콘은 앱 아이콘 하나를 정본으로(512×512 PNG) — 사본을 두면 앱 아이콘만 바뀌고 확장 아이콘은 옛것으로 남는다.
cp src-tauri/icons/icon.png "$STAGE/icon.png"

rm -f "$MCPB_FILE"
npx --no-install mcpb pack "$STAGE" "$MCPB_FILE" >/dev/null || die "mcpb pack 실패 (자세히: npx mcpb pack $STAGE)"
[[ -f "$MCPB_FILE" ]] || die "번들이 안 나왔다: $MCPB_FILE"

# ── 나온 물건을 열어서 확인한다 ──
VERIFY_DIR="$OUT_DIR/verify"
rm -rf "$VERIFY_DIR"
mkdir -p "$VERIFY_DIR"
unzip -qq "$MCPB_FILE" -d "$VERIFY_DIR" || die "번들 압축을 풀지 못했다"
for want in manifest.json server/ouro-mcp icon.png; do
  [[ -f "$VERIFY_DIR/$want" ]] || die "번들에 $want 가 없다"
done
# 🔴 맥오 금지. 실행 파일이 들어가는 순간 서명·공증 대상이 되는데, 맨 바이너리엔 티켓을 스테이플할 수 없어 «온라인일 때만 되는» 배포가 된다.
while IFS= read -r f; do
  if file -b "$f" | grep -q 'Mach-O'; then
    die "번들에 실행 파일이 들어 있다: ${f#"$VERIFY_DIR/"} — 이 번들은 서명·공증을 안 하는 전제다"
  fi
done < <(find "$VERIFY_DIR" -type f)
# 🔴 zip 이 실행 비트를 보존하는지. 깨지면 Claude 데스크톱이 런처를 못 띄워 «서버 시작 실패» 만 뜬다.
[[ -x "$VERIFY_DIR/server/ouro-mcp" ]] || die "압축을 푸니 런처에 실행 비트가 없다 — 이대로 배포하면 안 된다"
# 런처가 가리키는 이름 = 실제 사이드카 이름. 갈리면 앱은 멀쩡한데 확장만 «앱을 찾지 못했습니다».
grep -q 'Contents/MacOS/ouro-mcp' "$VERIFY_DIR/server/ouro-mcp" || die "런처가 Contents/MacOS/ouro-mcp 를 가리키지 않는다"
python3 -c '
import json
b=json.load(open("src-tauri/tauri.conf.json")).get("bundle",{}).get("externalBin",[])
assert "binaries/ouro-mcp" in b, b
' || die "tauri.conf.json 의 externalBin 에 binaries/ouro-mcp 가 없다 — 확장이 가리킬 바이너리가 안 생긴다"
# manifest 의 도구 목록 = 사이드카가 실제로 내는 도구. 갈리면 Claude 데스크톱 설치 화면이 거짓말을 한다.
python3 - <<'PY' || die "mcpb/manifest.json 의 tools 가 ouro-mcp 의 도구와 다르다"
import json, re
m = {t["name"] for t in json.load(open("mcpb/manifest.json"))["tools"]}
src = open("ouro-mcp/src/main.rs").read()
s = set(re.findall(r"#\[tool\([^]]*?\]\s*async fn (\w+)", src, re.S))
assert m == s, f"manifest {sorted(m)} != 사이드카 {sorted(s)}"
PY
rm -rf "$VERIFY_DIR" "$STAGE"

ok "$MCPB_FILE  $(du -h "$MCPB_FILE" | cut -f1)"
