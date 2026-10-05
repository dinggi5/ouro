#!/bin/bash
# 동기화 헬퍼(OuroSync.app) 개발 빌드 — Xcode 자동 서명(Apple Development, 개발 CloudKit 환경).
# 쓰는 법: ./scripts/build-sync-helper.sh
#   그다음 디버그 앱·테스트에 OURO_SYNC_HELPER=<출력 경로> 를 주면 헬퍼를 쓴다(sync.rs find_helper — 디버그 빌드만).
# 배포본에 넣는 일(Developer ID 프로파일·운영 CloudKit 스키마)은 아직 안 한다 — docs/RELEASE.md «동기화 헬퍼».
set -euo pipefail
cd "$(dirname "$0")/../sync"
command -v xcodegen >/dev/null || { echo "xcodegen 이 필요해요: brew install xcodegen" >&2; exit 1; }
xcodegen generate --quiet
xcodebuild -project OuroSync.xcodeproj -scheme OuroSync -configuration Debug -derivedDataPath build \
  -allowProvisioningUpdates -quiet build
echo "$PWD/build/Build/Products/Debug/OuroSync.app/Contents/MacOS/OuroSync"
