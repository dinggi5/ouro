#!/bin/bash
# iOS 앱 배포 빌드(개발 16) — archive → App Store Connect 용 export(.ipa). `--upload` 면 TestFlight 로 올린다.
#
#   ./scripts/upload-ios.sh            .ipa 만 만든다(ios/build/release/export) — 서명·프로비저닝 점검용
#   ./scripts/upload-ios.sh --upload   만든 뒤 App Store Connect 에 올린다(TestFlight 처리 후 내부 테스터에게 보인다)
#
# 처음 한 번(사람 몫): App Store Connect 에 앱 레코드(번들 id com.dinggi5.ouro.ios)가 있어야 올라간다.
# App ID 둘(.ios · .ios.widgets)·App Group·배포 프로파일은 -allowProvisioningUpdates 가 Xcode 에 로그인된 계정으로 만든다.
# 운영 환경은 export 가 정한다 — App Store 배포 프로파일이라 aps-environment 는 production, CloudKit 도 운영 환경이다.
# 올리기 인증은 맥 배포와 같은 App Store Connect API 키(scripts/release.env 의 APPLE_API_*)를 쓴다.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
UPLOAD=0
[[ "${1:-}" == "--upload" ]] && UPLOAD=1

# 빌드 번호 = 시각(분) — 올릴 때마다 커진다. 버전(1.0.0)은 project.yml.
BUILD="$(date +%Y%m%d%H%M)"
"$ROOT/scripts/build-parse.sh"
cd "$ROOT/ios"
command -v xcodegen >/dev/null || { echo "xcodegen 이 필요해요: brew install xcodegen" >&2; exit 1; }
xcodegen generate --quiet
OUT=build/release
rm -rf "$OUT"
mkdir -p "$OUT"
DEST=export
[[ $UPLOAD -eq 1 ]] && DEST=upload
# App Store Connect API 키 — 올리기뿐 아니라 **프로비저닝**(App ID·App Group·배포 프로파일 만들기)에도 쓴다.
# Xcode 에 계정이 로그인돼 있지 않아도 되게(개발 16 실측: 키체인의 Xcode 계정이 깨져 «No Accounts» 로 archive 가 죽었다).
# 자격증명은 메인 체크아웃의 release.env 에서(워크트리엔 없다 — release.sh 와 같은 규칙).
ENV_FILE="$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir)/../scripts/release.env"
[[ -f "$ENV_FILE" ]] || ENV_FILE="$ROOT/scripts/release.env"
AUTH=()
if [[ -f "$ENV_FILE" ]]; then
  KEY_ID="$(sed -n 's/^APPLE_API_KEY="\{0,1\}\([^"]*\)"\{0,1\}.*/\1/p' "$ENV_FILE" | head -1)"
  ISSUER="$(sed -n 's/^APPLE_API_ISSUER="\{0,1\}\([^"]*\)"\{0,1\}.*/\1/p' "$ENV_FILE" | head -1)"
  KEY_PATH="$(sed -n 's/^APPLE_API_KEY_PATH="\{0,1\}\([^"]*\)"\{0,1\}.*/\1/p' "$ENV_FILE" | head -1)"
  KEY_PATH="${KEY_PATH/#\$HOME/$HOME}"
  if [[ -n "$KEY_ID" && -n "$ISSUER" && -f "$KEY_PATH" ]]; then
    AUTH=(-authenticationKeyPath "$KEY_PATH" -authenticationKeyID "$KEY_ID" -authenticationKeyIssuerID "$ISSUER")
  fi
fi
if [[ $UPLOAD -eq 1 && ${#AUTH[@]} -eq 0 ]]; then
  echo "올리려면 release.env 에 APPLE_API_KEY · APPLE_API_ISSUER · APPLE_API_KEY_PATH 가 필요해요" >&2
  exit 1
fi
cat > "$OUT/ExportOptions.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>method</key><string>app-store-connect</string>
	<key>destination</key><string>$DEST</string>
	<key>teamID</key><string>74ZAMXKVXN</string>
	<key>signingStyle</key><string>automatic</string>
	<key>manageAppVersionAndBuildNumber</key><false/>
</dict>
</plist>
PLIST
xcodebuild -project Ouro.xcodeproj -scheme Ouro -configuration Release -destination "generic/platform=iOS" \
  -derivedDataPath build -archivePath "$OUT/Ouro.xcarchive" CURRENT_PROJECT_VERSION="$BUILD" \
  -allowProvisioningUpdates ${AUTH[@]+"${AUTH[@]}"} -quiet archive
xcodebuild -exportArchive -archivePath "$OUT/Ouro.xcarchive" -exportPath "$OUT/export" \
  -exportOptionsPlist "$OUT/ExportOptions.plist" -allowProvisioningUpdates ${AUTH[@]+"${AUTH[@]}"} -quiet
if [[ $UPLOAD -eq 1 ]]; then
  echo "올렸어요 — 1.0.0 ($BUILD). App Store Connect → TestFlight 에서 처리가 끝나면 보여요."
else
  ls "$OUT/export"/*.ipa
  echo "만들었어요 — 1.0.0 ($BUILD). 올리려면: ./scripts/upload-ios.sh --upload"
fi
