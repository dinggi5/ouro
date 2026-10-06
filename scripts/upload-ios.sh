#!/bin/bash
# iOS 앱 배포 빌드(개발 16) — archive → App Store Connect 용 export(.ipa). `--upload` 면 TestFlight 로 올린다.
#
#   ./scripts/upload-ios.sh            .ipa 만 만든다(ios/build/release/export) — 서명·프로비저닝 점검용
#   ./scripts/upload-ios.sh --upload   만든 뒤 App Store Connect 에 올린다(TestFlight 처리 후 내부 테스터에게 보인다)
#
# 처음 한 번(사람 몫): App Store Connect 에 앱 레코드(번들 id com.dinggi5.ouro.ios)가 있어야 올라간다 — 없으면
#   «App record … not found» 로 멈춘다. Xcode Organizer(open ios/build/release/Ouro.xcarchive)의 Distribute App 은 레코드를 만들어 준다.
# App ID 둘(.ios · .ios.widgets)·App Group·배포 프로파일은 -allowProvisioningUpdates 가 Xcode 에 로그인된 계정으로 만든다.
# 운영 환경은 export 가 정한다 — App Store 배포 프로파일이라 aps-environment 는 production, CloudKit 도 운영 환경이다.
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
# 인증은 **Xcode 에 로그인된 계정**(Xcode → 설정 → Accounts)이 한다 — 프로비저닝(App ID·App Group·배포 프로파일)과 올리기 둘 다.
# release.env 의 App Store Connect API 키는 공증용이라 프로파일을 만들 권한이 없다(개발 16 실측: «Authentication failed»).
# 권한 있는(Admin) 키를 따로 쓰려면 OURO_ASC_KEY_PATH · OURO_ASC_KEY_ID · OURO_ASC_ISSUER 를 준다.
AUTH=()
if [[ -n "${OURO_ASC_KEY_PATH:-}" ]]; then
  AUTH=(-authenticationKeyPath "$OURO_ASC_KEY_PATH" -authenticationKeyID "$OURO_ASC_KEY_ID" -authenticationKeyIssuerID "$OURO_ASC_ISSUER")
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
