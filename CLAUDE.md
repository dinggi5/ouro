# Ouro (우로우로) — 선순환 캘린더

맥 메뉴바 캘린더 + 로컬 AI + Claude/Codex 디스패처 + MCP. 기획은 `docs/PLAN.md`, 디자인은 `DESIGN.md`, 기록은 `DEVLOG.md`.

- 참고 구현: `~/프로젝트/지갑지갑` (Kura). 팝오버(`src-tauri/src/tray.rs`)·사이드카 빌드(`scripts/build-sidecars.sh`)·
  `.mcpb`·업데이트 파이프라인을 여기서 가져온다. 그 리포는 **읽기만** 한다.
- 데이터: `~/.ouro`. MCP 사이드카는 EventKit 을 직접 부르지 않는다(권한이 Claude 앱에 붙는다) — 소켓으로 앱에 묻는다.
- 불변 규칙: 남이 보낸 일정은 부탁이 될 수 없다 / AI 가 만든 부탁은 사람 승인 전엔 안 돈다 / 바깥으로 나간 원문은 저장한다.
- 날짜 계산은 LLM 이 아니라 결정적 파서가 한다.
