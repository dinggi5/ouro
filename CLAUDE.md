# Ouro (우로우로) — 선순환 캘린더

맥 메뉴바 캘린더(로컬 저장) + Claude/Codex 디스패처 + MCP. 로컬 AI 모델은 MVP 에서 뺐다(사장이 정할 때 추가). 기획은 `docs/PLAN.md`, 디자인은 `DESIGN.md`, 기록은 `DEVLOG.md`.

- 참고 구현: `~/프로젝트/지갑지갑` (Kura). 팝오버(`src-tauri/src/tray.rs`)·사이드카 빌드(`scripts/build-sidecars.sh`)·
  `.mcpb`·업데이트 파이프라인을 여기서 가져온다. 그 리포는 **읽기만** 한다.
- 오픈소스(MIT) · macOS 26+ · 번들 id `com.dinggi5.ouro`.
- 스택: Tauri 2 (확정). 캘린더는 **앱 자체 캘린더** — 일정·부탁·답 전부 `~/.ouro/ouro.db`(SQLite). 맥 기본 캘린더(EventKit)는 v0.2 이후 선택 기능.
- MCP 사이드카(`ouro-mcp/`, 독립 크레이트)는 DB 를 직접 열지 않는다 — 소켓 `~/.ouro/ouro.sock` 으로 앱(`src-tauri/src/mcp.rs`)에 묻는다.
  소켓엔 **승인 문이 없다** — AI 제안이 일정이 되는 길은 팝오버의 `approve_proposal` 하나.
- iCloud 동기화(개발 10): CloudKit 은 헬퍼 앱 `sync/`(OuroSync.app, Swift 패키지 OuroSyncKit — iOS 와 공용)가 맡고 앱(`src-tauri/src/sync.rs`)과 표준입출력 JSON 줄로만 말한다.
  헬퍼도 DB 를 안 연다. 바뀐 줄은 스키마 9 트리거가 `sync_outbox` 에 적는다. 동기화 중 예약 부탁은 «실행 맥» 한 대만 돌린다.
  헬퍼 개발 빌드 `./scripts/build-sync-helper.sh`(Xcode 자동 서명, 컨테이너 `iCloud.com.dinggi5.ouro`) → 디버그 앱·테스트에 `OURO_SYNC_HELPER=<경로>`.
  🔴 CloudKit 은 변경을 만든 **기기**에 그 변경을 다시 주지 않는다 — 같은 맥의 두 프로세스로는 «이어 받기» 를 검증할 수 없다.
- `src-tauri` 를 빌드·테스트하기 전에 `./scripts/build-sidecars.sh` (externalBin 이라 사이드카 파일이 없으면 `cargo test` 도 실패한다).
- 불변 규칙: 바깥에서 들어온 글(.ics·AI 답)은 부탁 문장이 될 수 없다 / AI 가 만든 부탁은 사람 승인 전엔 안 돈다 / 바깥으로 나간 원문은 저장한다.
- 날짜 계산은 LLM 이 아니라 결정적 파서가 한다.
- 원격: `backup` = 비공개 `dinggi5/ouro-dev`(`DEVLOG.md` 포함, 세션 끝에 `git push backup main`). `origin` 이라는 이름은 쓰지 않는다(사이드바 묶음 이름이 바뀐다).
  `public` = 공개 `dinggi5/ouro`(개발 8 부터) — `scripts/publish-public.sh` 가 일지를 이력에서 걸러 올린다. 커밋 해시가 달라서 `git fetch public` 은 `--no-tags` 로.
- 배포: 버전 여섯 곳 + `docs/release-notes/vX.Y.Z.md` → main 머지·`git push backup main` → 메인 체크아웃에서 `./scripts/release.sh --publish`(`docs/RELEASE.md`).
  업데이트 서명 키 `~/.tauri/ouro-updater.key` 는 잃으면 기존 사용자가 영영 업데이트를 못 받는다.
- 최소 macOS 26 의 정본은 `src-tauri/Info.plist`(LSMinimumSystemVersion). `tauri.conf.json` 의 minimumSystemVersion 은 **일부러 11.0** —
  Xcode 27 링커가 배포 타깃 26 으로 만든 proc-macro dylib 을 dyld 가 거부한다(Info.plist 주석).
