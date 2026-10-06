#!/bin/bash
# 동기화 헬퍼(OuroSync.app) 빌드.
#
#   ./scripts/build-sync-helper.sh            개발 빌드 — Xcode 자동 서명(Apple Development, 개발 CloudKit 환경).
#                                             디버그 앱·테스트에 OURO_SYNC_HELPER=<출력 경로> 를 주면 쓴다(sync.rs find_helper — 디버그 빌드만).
#   ./scripts/build-sync-helper.sh --release  배포 빌드(개발 16) — archive → Developer ID 로 export →
#                                             src-tauri/helpers/OuroSync.app. tauri.conf.json 의 bundle.macOS.files 가
#                                             그걸 Ouro.app/Contents/Helpers/ 로 넣는다(release.sh 가 컴파일 전에 부른다).
#
# 배포 빌드의 운영 환경은 엔타이틀먼트 파일이 아니라 **export 가** 바꾼다:
#   · aps-environment       — Developer ID 프로파일이 production 이라 export 가 production 으로 다시 서명한다.
#   · icloud-container-environment — ExportOptions 의 iCloudContainerEnvironment=Production 이 넣는다
#     (Developer ID 앱은 이게 없으면 개발 CloudKit 에 붙는다 — 사용자 데이터가 개발 DB 로 간다).
# 결과물은 release.sh 가 다시 확인한다(«헬퍼 검증»). Tauri 는 files 로 넣은 번들을 다시 서명하지 않는다 —
# 본체 서명(--deep 없음)이 헬퍼의 서명·엔타이틀먼트를 그대로 둔다(tauri-bundler macos/app.rs, 2.12 실측 읽음).
#
# 처음 한 번: Developer ID 프로비저닝 프로파일이 없으면 -allowProvisioningUpdates 가 Xcode 에 로그인된 계정으로 만든다.
set -euo pipefail
cd "$(dirname "$0")/../sync"
command -v xcodegen >/dev/null || { echo "xcodegen 이 필요해요: brew install xcodegen" >&2; exit 1; }
xcodegen generate --quiet

if [[ "${1:-}" != "--release" ]]; then
  xcodebuild -project OuroSync.xcodeproj -scheme OuroSync -configuration Debug -derivedDataPath build \
    -allowProvisioningUpdates -quiet build
  echo "$PWD/build/Build/Products/Debug/OuroSync.app/Contents/MacOS/OuroSync"
  exit 0
fi

ARCHIVE="build/release/OuroSync.xcarchive"
EXPORT="build/release/export"
OUT="../src-tauri/helpers/OuroSync.app"
rm -rf build/release
mkdir -p build/release
cat > build/release/ExportOptions.plist <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>method</key><string>developer-id</string>
	<key>teamID</key><string>74ZAMXKVXN</string>
	<key>signingStyle</key><string>automatic</string>
	<key>iCloudContainerEnvironment</key><string>Production</string>
</dict>
</plist>
PLIST
xcodebuild -project OuroSync.xcodeproj -scheme OuroSync -configuration Release -derivedDataPath build \
  -archivePath "$ARCHIVE" -allowProvisioningUpdates -quiet archive
xcodebuild -exportArchive -archivePath "$ARCHIVE" -exportPath "$EXPORT" \
  -exportOptionsPlist build/release/ExportOptions.plist -allowProvisioningUpdates -quiet

APP="$EXPORT/OuroSync.app"
[[ -d "$APP" ]] || { echo "export 결과에 OuroSync.app 이 없어요: $EXPORT" >&2; exit 1; }
# 빌드한 자리에서 바로 확인한다 — 운영 환경이 아니면 여기서 멈춘다(release.sh 도 앱 안에서 다시 본다).
ENT="$(codesign -d --entitlements :- "$APP" 2>/dev/null)"
grep -A1 'icloud-container-environment' <<<"$ENT" | grep -q 'Production' \
  || { echo "헬퍼가 운영 CloudKit 환경이 아니에요(icloud-container-environment)" >&2; exit 1; }
grep -A1 'aps-environment' <<<"$ENT" | grep -q 'production' \
  || { echo "헬퍼의 aps-environment 가 production 이 아니에요" >&2; exit 1; }
rm -rf "$OUT"
mkdir -p "$(dirname "$OUT")"
# ditto — 번들 안 심볼릭 링크·확장 속성을 그대로 옮긴다(cp -R 은 서명을 깰 수 있다).
ditto "$APP" "$OUT"
codesign --verify --strict --deep "$OUT"
echo "$(cd "$(dirname "$OUT")" && pwd)/OuroSync.app"
