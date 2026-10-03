# Ouro (우로우로)

**이 맥에만 저장되는 메뉴바 캘린더 — 정해진 시간에 Claude Code·Codex 에게 일을 «부탁» 하고, 답을 다시 캘린더에 걸어 둬요.**

- 일정은 `~/.ouro/ouro.db`(SQLite) 한 파일에만 있어요. 계정·서버·클라우드 없음.
- **부탁**: «매일 9시, 오늘 할 일 정리해 줘 — 코덱스한테» 를 걸어 두면 그 시간에 내 맥의 `claude`·`codex` 가 돌고, 답이 그 칸에 남아요.
- **MCP**: Claude Code·Claude 데스크톱이 일정을 읽고 빈 시간을 찾고, 일정·부탁을 **제안**해요. 제안은 팝오버 카드를 사람이 승인해야만 일정·부탁이 돼요.
- 오픈소스(MIT). macOS 26 이상, Apple Silicon.

In English: see [below](#english).

---

## 설치

### Homebrew

```bash
brew install --cask dinggi5/tap/ouro
```

### DMG

[Releases](https://github.com/dinggi5/ouro/releases/latest) 에서 `Ouro_<버전>_aarch64.dmg` 를 받아 `Ouro.app` 을 응용 프로그램 폴더로 옮겨요.
Developer ID 로 서명하고 애플 공증을 받은 배포본이라 경고 없이 열려요. 직접 확인하려면:

```bash
codesign -dv --verbose=2 /Applications/Ouro.app 2>&1 | grep -E 'Identifier|TeamIdentifier'
# Identifier=com.dinggi5.ouro
# TeamIdentifier=74ZAMXKVXN
spctl -a -vv /Applications/Ouro.app   # source=Notarized Developer ID
```

켜면 메뉴바에 빈 원(◯) 아이콘이 생겨요(도크 아이콘은 없어요). 왼쪽 클릭 = 열기, 오른쪽 클릭 = 메뉴(업데이트 확인·종료).
업데이트는 앱이 스스로 확인하고, 노트를 보여 준 뒤 사람이 눌러야 설치해요.

## 부탁 쓰기

부탁은 내 맥에 깔린 CLI 로 돌아요 — 쓰려는 쪽을 깔고 한 번 로그인해 두세요.

- Claude Code: `claude` ([설치](https://docs.claude.com/en/docs/claude-code))
- Codex: `codex` ([설치](https://github.com/openai/codex))

부탁은 **대화만** 해요. 파일 읽기·쓰기, 셸, 브라우저, 컴퓨터 조작, 하위 에이전트는 꺼진 채로 돌고, 웹 검색은 부탁마다 고를 수 있어요.
보낸 글과 받은 답은 그대로 DB 에 남아요. 한 가지 알아 둘 것: Codex 는 `~/.codex/AGENTS.md`(내 전역 지침)를 늘 함께 싣고 가요 — 부탁 화면에도 적혀 있어요.

## AI 연결 (MCP)

MCP 서버는 앱 안에 들어 있어요(`Ouro.app/Contents/MacOS/ouro-mcp`). 앱이 켜져 있어야 동작해요.

**Claude Code**

```bash
claude mcp add -s user ouro /Applications/Ouro.app/Contents/MacOS/ouro-mcp
```

**Codex**

```bash
codex mcp add ouro -- /Applications/Ouro.app/Contents/MacOS/ouro-mcp
```

**Claude 데스크톱** — 릴리스의 `ouro-<버전>.mcpb` 를 받아 더블클릭하거나 설정 → 확장에서 고르세요.
이 확장에는 실행 파일이 없어요. 설치된 `Ouro.app` 의 서명(팀 `74ZAMXKVXN`)을 확인한 뒤 그 안의 `ouro-mcp` 를 실행해요.

AI 가 할 수 있는 것: 일정 읽기(`get_agenda`) · 빈 시간 찾기(`find_free_time`) · 일정 제안(`propose_event`) · 부탁 제안(`propose_errand`) · 제안 상태 보기.
**할 수 없는 것**: 일정을 직접 넣거나 고치거나 지우기, 부탁을 직접 돌리기. 비공개로 표시한 일정은 AI 에게 안 보여요.

## 무엇이 맥 밖으로 나가나

- 부탁의 글(과 이어서 묻는 대화)은 그 부탁을 받는 쪽(Anthropic 또는 OpenAI)으로 내 CLI 를 통해 나가요.
- MCP 로 붙은 AI 에게는 그 AI 가 물은 기간의 일정(비공개 제외)이 가요.
- 업데이트 확인 때 GitHub 에서 `latest.json` 을 받아요.
- 그 밖엔 없어요. 분석·추적 없음.

## 지우기

```bash
brew uninstall --cask ouro        # 또는 Ouro.app 을 휴지통으로
```

앱을 지워도 **캘린더는 남아요** — `~/.ouro/`(일정 DB 와 7일치 백업)는 일부러 안 지워요(`--zap` 도 안 지워요).
정말 다 지우려면 `rm -rf ~/.ouro` 를 직접 실행하세요. 되돌릴 수 없어요.

## 소스에서 빌드

```bash
npm ci
./scripts/build-sidecars.sh
npm run tauri dev
```

Rust(stable)·Node 20.19+ 필요. 배포본 만들기는 [docs/RELEASE.md](docs/RELEASE.md).
태그 `v<버전>` 을 체크아웃하면 그 배포본과 같은 소스예요.

---

## English

**Ouro is a menu bar calendar for macOS that stores everything on this Mac only — and can schedule "errands": a message your own Claude Code or Codex CLI runs at a set time, with the answer kept on the calendar.**

- Install: `brew install --cask dinggi5/tap/ouro` (macOS 26+, Apple Silicon), or the notarized DMG from Releases.
- Errands run your locally installed `claude` / `codex` in conversation-only mode (no file, shell, browser, or computer-use tools).
- MCP: `claude mcp add -s user ouro /Applications/Ouro.app/Contents/MacOS/ouro-mcp`. AI clients can read events, find free time, and **propose** events or errands — nothing is added or run until you approve the card in the popover. Private events are never shared.
- Data lives in `~/.ouro/ouro.db`. Uninstalling the app keeps it; delete `~/.ouro` yourself if you want it gone.
- License: MIT.
