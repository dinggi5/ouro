# Ouro (우로우로) — 선순환 캘린더

맥 메뉴바 캘린더(로컬 저장) + Claude/Codex 디스패처 + MCP. 로컬 AI 모델은 MVP 에서 뺐다(사장이 정할 때 추가). 기획은 `docs/PLAN.md`, 디자인은 `DESIGN.md`, 기록은 `DEVLOG.md`.

- 참고 구현: `~/프로젝트/지갑지갑` (Kura). 팝오버(`src-tauri/src/tray.rs`)·사이드카 빌드(`scripts/build-sidecars.sh`)·
  `.mcpb`·업데이트 파이프라인을 여기서 가져온다. 그 리포는 **읽기만** 한다.
- 오픈소스(MIT) · macOS 26+ · 번들 id `com.dinggi5.ouro`.
- 스택: Tauri 2 (확정). 캘린더는 **앱 자체 캘린더** — 일정·부탁·답 전부 `~/.ouro/ouro.db`(SQLite). 맥 기본 캘린더(EventKit)는 v0.2 이후 선택 기능.
- MCP 사이드카는 DB 를 직접 열지 않는다 — 소켓으로 앱에 묻는다(쓰는 곳은 앱 하나).
- 불변 규칙: 바깥에서 들어온 글(.ics·AI 답)은 부탁 문장이 될 수 없다 / AI 가 만든 부탁은 사람 승인 전엔 안 돈다 / 바깥으로 나간 원문은 저장한다.
- 날짜 계산은 LLM 이 아니라 결정적 파서가 한다.
