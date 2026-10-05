#!/bin/bash
# 날짜 파서(ouro-parse, 러스트)를 iOS·맥용 xcframework 로 묶는다(개발 14 — 파서 하나 원칙).
# 결과: ouro-parse/swift/OuroParseFFI.xcframework (커밋하지 않는다, .gitignore). 스위프트 패키지 `OuroParse` 가 이걸 감싼다.
# scripts/build-ios.sh 가 먼저 부른다. 맥 조각(aarch64-apple-darwin)은 `swift test`(ouro-parse/swift)용이다.
# 처음 한 번: rustup target add aarch64-apple-ios aarch64-apple-ios-sim
set -euo pipefail
cd "$(dirname "$0")/../ouro-parse"
TARGETS=(aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-darwin)
installed=$(rustup target list --installed)
for t in "${TARGETS[@]}"; do
  grep -qx "$t" <<<"$installed" || { echo "러스트 타깃이 없어요: rustup target add $t" >&2; exit 1; }
done
for t in "${TARGETS[@]}"; do
  cargo build --quiet --release --features ffi --lib --target "$t"
done
OUT=swift/OuroParseFFI.xcframework
rm -rf "$OUT"
args=()
for t in "${TARGETS[@]}"; do args+=(-library "target/$t/release/libouro_parse.a" -headers swift/include); done
xcodebuild -create-xcframework "${args[@]}" -output "$OUT" >/dev/null
echo "✓ $OUT"
