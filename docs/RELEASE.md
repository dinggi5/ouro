# 배포본 만들기 — 서명·공증·업데이트·캐스크

`scripts/release.sh` 한 줄이 서명(Developer ID) → 공증 → 스테이플 → 업데이트 산출물 → `latest.json` → `.mcpb` →
(`--publish` 면) 공개 소스 → 태그 → GitHub 릴리스 → 업로드본 재검증 → Homebrew 캐스크까지 한다.
스크립트는 지갑지갑(Kura)의 것을 가져왔다 — 게이트 하나하나가 그쪽에서 실제로 난 사고의 흔적이라, 이유는 스크립트 주석에 있다.

## 1회 설정 (개발 8 에 끝냄)

| 것 | 어디 | 메모 |
|---|---|---|
| Developer ID Application 인증서 | 로그인 키체인 | Kura 와 같은 개발자 계정(팀 `74ZAMXKVXN`) |
| App Store Connect API 키(.p8) | `release.env` 의 `APPLE_API_KEY_PATH` | Kura 와 같은 키 |
| **업데이트 서명 키** | `~/.tauri/ouro-updater.key` (+ 암호는 `release.env`) | 🔴 **Ouro 전용.** 잃으면 기존 사용자 전원이 다음 업데이트를 영영 못 받는다. 새면 이미 깔린 Ouro 에 임의 코드를 밀 수 있다. 키 파일과 암호를 함께 오프라인·비밀번호 관리자에 백업할 것 |
| `scripts/release.env` | **메인 체크아웃**(`~/프로젝트/Ouro/scripts/`), chmod 600 | `.gitignore` 로 빠진다. 워크트리에서 돌리면 스크립트가 메인 것을 읽는다. 본보기는 `scripts/release.env.example` |
| `public` 원격 | `git remote add public https://github.com/dinggi5/ouro.git` | 공개 오픈소스 리포. `origin` 이라는 이름은 쓰지 않는다(CLAUDE.md) |
| Homebrew tap | `brew tap dinggi5/tap` | 캐스크가 없으면 첫 `--publish` 가 `packaging/homebrew/ouro.rb` 로 만든다 |

## 원격이 둘인 이유

- `backup` = 비공개 `dinggi5/ouro-dev`. 개발 일지(`DEVLOG.md`)까지 전부.
- `public` = 공개 `dinggi5/ouro`. `scripts/publish-public.sh` 가 `backup/main` 을 복제해 **이력 전체에서 일지를 뺀** 사본을 민다.
  같은 입력이면 같은 해시가 나와서 매번 빨리감기로 이어진다(강제 푸시 없음).
- 그래서 공개 쪽 커밋 해시는 개발 커밋과 다르다. 태그는 «HEAD 의 트리에서 `DEVLOG.md` 만 뺀 트리» 를 가진 공개 커밋에 찍힌다
  (`release.sh` 8-0). 같은 이름의 태그가 `backup` 에선 개발 커밋을 가리킨다. 🔴 `git fetch public` 은 `--no-tags` 로 — 태그를 받아 오면
  로컬 태그와 부딪친다.

## 배포 순서

```bash
# 1. 버전을 여섯 곳에서 올린다 (사전 점검이 불일치를 잡는다)
#    src-tauri/tauri.conf.json / package.json / package-lock.json / src-tauri/Cargo.toml / ouro-mcp/Cargo.toml / mcpb/manifest.json
#    + sync/project.yml(헬퍼) · ios/project.yml 둘(앱·위젯) 의 CFBundleShortVersionString — 사전 점검이 같이 본다(개발 16)
#    (package.json·lock 은 `npm version X.Y.Z --no-git-tag-version`, Cargo.lock 은 `cargo update -p ouro --offline` 로 따라온다)
# 2. 릴리스 노트 — docs/release-notes/vX.Y.Z.md (앱의 업데이트 카드와 릴리스 페이지에 그대로 뜬다)
# 3. 커밋 → main 에 ff 머지 → git push backup main
# 4. 빌드·검증·배포 한 번에 (메인 체크아웃에서)
./scripts/release.sh --publish
```

키체인 «항상 허용» 팝업이 뜨면 허용한다(헤드리스면 서명이 그냥 실패한다). 끊겼으면 `--resume-publish`(지문 대조 후 배포 단계만).

## 확인

```bash
brew install --cask dinggi5/tap/ouro
codesign -dv --verbose=2 /Applications/Ouro.app 2>&1 | grep -E 'Identifier|TeamIdentifier'
spctl -a -vv /Applications/Ouro.app          # source=Notarized Developer ID
curl -sL https://github.com/dinggi5/ouro/releases/latest/download/latest.json | head
```

## 아직 안 한 것

- 유니버설(인텔) 빌드 배포 — 캐스크가 `_aarch64.dmg`·`arch: :arm64` 로 고정돼 있어 `--publish --universal` 은 막혀 있다.
- 재현 가능 빌드 — 같은 커밋도 서명 타임스탬프·공증 티켓 때문에 바이트가 다르다.
- `.mcpb` 서명 — mcpb 2.1.2 의 서명본을 Claude 가 거부하는 버그(modelcontextprotocol/mcpb#278). 릴리스 본문에 sha256 만 싣는다.

## 동기화 헬퍼 (개발 16)

`release.sh` 가 컴파일 전에 `./scripts/build-sync-helper.sh --release` 를 부른다: archive → **Developer ID export**
(`iCloudContainerEnvironment=Production`, 프로파일이 `aps-environment=production`) → `src-tauri/helpers/OuroSync.app`(gitignore) →
`bundle.macOS.files` 가 `Ouro.app/Contents/Helpers/` 로 넣는다. 번들러는 이 헬퍼를 다시 서명하지 않는다. «동기화 헬퍼 검증» 단계가
팀·번들 ID·Developer ID·타임스탬프·하드닝·운영 엔타이틀먼트·프로파일·버전을 본다 — 🔴 운영이 아니면 사용자 일정이 개발 CloudKit DB 로 간다.

- **스키마 정본 `sync/schema.ckdb`**(개발 16): 개발 환경 스키마는 «앱이 실제로 저장해 본 칸» 만 생겨서 반쪽이었다(Run·Proposal·ErrandProposal 통째로 없음).
  칸을 바꾸면 이 파일도 고치고(`Schema.swift` 와 같은 목록) 콘솔 «Import Schema…» 로 개발 환경에 넣은 뒤 운영 배포.
- **CloudKit 운영 스키마는 사람이 배포한다** — CloudKit Console(icloud.developer.apple.com) → `iCloud.com.dinggi5.ouro` →
  «Deploy Schema Changes…» (개발 → 운영). 레코드 종류가 늘 때마다(새 칸 포함) 다시. 안 하면 운영에서 저장이 «알 수 없는 종류» 로 실패한다.

## iOS (개발 16)

`./scripts/upload-ios.sh` = archive → App Store Connect export(.ipa), `--upload` 면 TestFlight 로. 빌드 번호는 시각(분).
- 처음 한 번(사람): App Store Connect 에 앱 레코드(번들 id `com.dinggi5.ouro.ios`) 만들기.
- 프로비저닝(App ID 둘 `.ios`·`.ios.widgets` · App Group `group.com.dinggi5.ouro` · 배포 프로파일)은 `-allowProvisioningUpdates` 가 만든다 —
  **Xcode → 설정 → 계정에 Apple ID 가 로그인돼 있어야 한다**. 개발 16 실측: 키체인의 Xcode 계정이 깨져(«No Accounts») 실패했고,
  release.env 의 API 키로는 «Authentication failed» — 공증용 키라 프로파일을 만들 권한(Admin)이 없는 것으로 보인다.
