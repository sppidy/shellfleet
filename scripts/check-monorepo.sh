#!/bin/sh
set -eu

script_path=$0
case $script_path in
    /*) ;;
    *) script_path=./$script_path ;;
esac
repo_root=$(CDPATH= cd -P "$(dirname "$script_path")/.." && pwd)
cd "$repo_root"

fail() {
    printf 'monorepo invariant failed: %s\n' "$*" >&2
    exit 1
}

test ! -e .gitmodules || fail '.gitmodules must not exist'

gitlinks=$(git ls-files --stage | awk '$1 == "160000" { print $4 }')
test -z "$gitlinks" || fail "gitlinks are forbidden: $gitlinks"

candidate_files=$(git ls-files --cached --others --exclude-standard | while IFS= read -r path; do
    test ! -e "$path" || printf '%s\n' "$path"
done)

nested_locks=$(printf '%s\n' "$candidate_files" | awk '/\/Cargo\.lock$/ { print }')
test -z "$nested_locks" || fail "Cargo.lock must live only at the root: $nested_locks"

nested_github=$(printf '%s\n' "$candidate_files" | awk '
    $0 ~ /(^|\/)\.github\// && $0 !~ /^\.github\// { print }
')
test -z "$nested_github" || fail "GitHub automation must live at the root: $nested_github"

private_paths=$(printf '%s\n' "$candidate_files" | awk '
    $0 == "Dockerfile.ee" ||
    $0 == "docker-compose.ee.yml" ||
    $0 == "ee-docs.html" ||
    $0 ~ /^ee\// { print }
')
test -z "$private_paths" || fail "private EE source entered the public tree: $private_paths"

legacy_urls=$(git grep -En \
    'github\.com/sppidy/(shellfleet-agent|shellfleet-cli|shellfleet-server|shellfleet-shared|shellfleet-web)(\.git)?' \
    -- . ':!scripts/check-monorepo.sh' || true)
test -z "$legacy_urls" || fail "legacy component repository URLs remain: $legacy_urls"

printf 'monorepo invariants: ok\n'
