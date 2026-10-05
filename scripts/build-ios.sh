#!/bin/bash
# iOS 앱 시뮬레이터 빌드 → 켜기(개발 11).
# 쓰는 법: ./scripts/build-ios.sh            # 실제 동기화(시뮬레이터가 iCloud 에 로그인돼 있어야 맥과 맞춘다)
#         ./scripts/build-ios.sh -demo      # 메모리에만 있는 예시 하루(화면 확인용)
# 🔴 빌드 SDK 와 같은 메이저 이상의 iOS 시뮬레이터에서 켠다(전역 규칙 — Xcode 27 = iOS 27).
set -euo pipefail
cd "$(dirname "$0")/../ios"
command -v xcodegen >/dev/null || { echo "xcodegen 이 필요해요: brew install xcodegen" >&2; exit 1; }
SIM="${OURO_SIM:-iPhone 17}"
xcodegen generate --quiet
xcodebuild -project Ouro.xcodeproj -scheme Ouro -destination "platform=iOS Simulator,name=$SIM" -derivedDataPath build -quiet build
APP=build/Build/Products/Debug-iphonesimulator/Ouro.app
SDK=$(/usr/libexec/PlistBuddy -c "Print :DTPlatformVersion" "$APP/Info.plist" | cut -d. -f1)
RUNTIME=$(xcrun simctl list runtimes | grep -o 'iOS [0-9]*' | awk '{print $2}' | sort -n | tail -1)
if [ "${RUNTIME:-0}" -lt "$SDK" ]; then
  echo "iOS $SDK 시뮬레이터 런타임이 없어요(있는 최신: $RUNTIME) — xcodebuild -downloadPlatform iOS" >&2
  exit 1
fi
xcrun simctl boot "$SIM" 2>/dev/null || true
xcrun simctl install "$SIM" "$APP"
xcrun simctl terminate "$SIM" com.dinggi5.ouro.ios 2>/dev/null || true
xcrun simctl launch "$SIM" com.dinggi5.ouro.ios "$@"
open "$(xcode-select -p)/Applications/Simulator.app" 2>/dev/null || true
