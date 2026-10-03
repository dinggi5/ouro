cask "ouro" do
  version "0.0.0"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"

  url "https://github.com/dinggi5/ouro/releases/download/v#{version}/Ouro_#{version}_aarch64.dmg"
  name "Ouro"
  desc "Menu bar calendar that schedules errands for Claude Code and Codex"
  homepage "https://github.com/dinggi5/ouro"

  livecheck do
    url :url
    strategy :github_latest
  end

  # 앱이 스스로 업데이트한다(0.1.0~). brew 가 업그레이드를 맡지 않게.
  auto_updates true
  # Apple Silicon 전용 빌드. Intel 맥에서 조용히 설치돼 안 열리는 것보다 설치 단계에서 막히는 편이 낫다.
  depends_on arch: :arm64
  depends_on macos: ">= :tahoe"

  app "Ouro.app"

  # 메뉴바 상주 앱이라 지우기 전에 끈다.
  uninstall quit: "com.dinggi5.ouro"

  # ⚠️ "~/.ouro" 를 넣지 않는다 — 의도적이다. 일정·부탁·답의 유일한 원본(ouro.db)과 백업이 거기 있다.
  # `brew uninstall --zap` 한 줄과 «캘린더가 통째로 없어짐» 사이에 확인 절차가 없다. README «지우기» 에 손으로 지우는 법을 적었다.
  zap trash: [
    "~/Library/Caches/com.dinggi5.ouro",
    "~/Library/HTTPStorages/com.dinggi5.ouro",
    "~/Library/Saved Application State/com.dinggi5.ouro.savedState",
    "~/Library/WebKit/com.dinggi5.ouro",
  ]
end
