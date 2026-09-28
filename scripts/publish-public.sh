#!/usr/bin/env bash
# 공개 저장소로 내보내기 — 개발 일지(DEVLOG.md)를 **이력 전체에서** 뺀 main 을 `public` 원격에 올린다.
#
# 저장소가 둘이다(사장, 개발 2):
#   origin  = 비공개 백업. 일지까지 전부. 세션마다 여기로 푸시한다.
#   public  = 공개 오픈소스(MIT). 일지는 개인 운영 기록이라 싣지 않는다.
#
# 매번 origin/main 을 새로 복제해 같은 필터를 돌린다. filter-branch 는 작성자·날짜를 그대로 두므로
# **같은 입력이면 같은 커밋 해시**가 나온다 → 공개 쪽은 매번 빨리감기(fast-forward)로 이어진다.
# 그래서 강제 푸시를 쓰지 않는다: 해시가 어긋나 빨리감기가 안 되면 멈추고 사람이 본다.
#
# 쓰는 법:  scripts/publish-public.sh            (확인만 — 무엇이 올라갈지 보여 준다)
#           scripts/publish-public.sh --push     (실제로 올린다)
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
PUBLIC_URL="$(git remote get-url public 2>/dev/null || true)"
if [[ -z "$PUBLIC_URL" ]]; then
  echo "public 원격이 없어요: git remote add public https://github.com/dinggi5/ouro.git" >&2
  exit 1
fi
ORIGIN_URL="$(git remote get-url origin)"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

git clone --quiet --single-branch --branch main "$ORIGIN_URL" "$WORK/repo"
cd "$WORK/repo"
FILTER_BRANCH_SQUELCH_WARNING=1 git filter-branch --prune-empty \
  --index-filter 'git rm --cached --ignore-unmatch --quiet DEVLOG.md' -- main >/dev/null 2>&1

# 마지막 확인 — 올라갈 main 의 어느 커밋에도 일지가 남아 있으면 안 된다.
# (`--all` 은 안 된다: filter-branch 가 옛 이력을 refs/original 에 남긴다. `| grep -q` 도 안 된다:
#  pipefail 아래서 grep 이 먼저 끝나면 git log 가 SIGPIPE 로 실패해 검사가 조용히 통과한다 — 실측.)
if [[ -n "$(git log main --format=%H -- DEVLOG.md)" ]]; then
  echo "🔴 걸러 낸 이력에 DEVLOG.md 가 남았어요. 올리지 않습니다." >&2
  exit 1
fi

echo "공개로 나갈 main: $(git rev-parse --short main) — $(git log -1 --format=%s main)"
echo "커밋 $(git rev-list --count main)개, 파일 $(git ls-files | wc -l | tr -d ' ')개"

if [[ "${1:-}" == "--push" ]]; then
  git push "$PUBLIC_URL" main:main
else
  echo "(확인만 했어요. 올리려면 --push)"
fi
