#!/usr/bin/env bash
# Builds the demo repositories used for website screenshots:
#   scripts/demo-repos.sh <dir>   → <dir>/aurora, <dir>/nebula-api, <dir>/orbit-docs
# aurora has feature branches, merges, tags, a remote, a worktree and two
# uncommitted changes. Everything is dated two days ago.
set -euo pipefail

out=${1:?usage: demo-repos.sh <dir>}
rm -rf "$out"
mkdir -p "$out"
out=$(cd "$out" && pwd)
when=$(( $(date +%s) - 2 * 86400 ))
tick=0

commit() { # author email message [file]
  local file=${4:-src/$(echo "$3" | tr 'A-Z ' 'a-z_' | tr -cd 'a-z_').rs}
  mkdir -p "$(dirname "$file")"
  echo "// $3" >> "$file"
  git add -A
  tick=$((tick + 60))
  GIT_AUTHOR_NAME=$1 GIT_AUTHOR_EMAIL=$2 GIT_COMMITTER_NAME=$1 GIT_COMMITTER_EMAIL=$2 \
    GIT_AUTHOR_DATE="@$((when + tick)) +0000" GIT_COMMITTER_DATE="@$((when + tick)) +0000" \
    git commit -q -m "$3"
}

merge() { # author email branch
  tick=$((tick + 60))
  GIT_AUTHOR_NAME=$1 GIT_AUTHOR_EMAIL=$2 GIT_COMMITTER_NAME=$1 GIT_COMMITTER_EMAIL=$2 \
    GIT_AUTHOR_DATE="@$((when + tick)) +0000" GIT_COMMITTER_DATE="@$((when + tick)) +0000" \
    git merge -q --no-ff -m "Merge branch '$3'" "$3"
}

ada=("Ada Park" ada@aurora.dev)
mira=("Mira Chen" mira.chen@aurora.dev)
leo=("Leo Hart" leo@aurora.dev)
sam=("Sam Okafor" sam@aurora.dev)

git init -q -b main "$out/aurora"
cd "$out/aurora"
git config commit.gpgsign false
commit "${ada[@]}" "Initial project layout" README.md
commit "${leo[@]}" "Config loader"
commit "${ada[@]}" "HTTP server skeleton"
git tag v0.9.0
git checkout -q -b feature/auth
commit "${mira[@]}" "Session store"
commit "${mira[@]}" "Login and logout endpoints"
commit "${mira[@]}" "Refresh tokens + tests"
git checkout -q main
commit "${leo[@]}" "Structured logging"
merge "${mira[@]}" feature/auth
commit "${ada[@]}" "Bump dependencies" Cargo.toml
commit "${leo[@]}" "Dark mode" ui/theme.rs
git tag v1.0.0
git checkout -q -b feature/search
commit "${sam[@]}" "Index documents on save"
commit "${sam[@]}" "Fuzzy matching for search"
commit "${sam[@]}" "Highlight matches in results"
git checkout -q main
merge "${leo[@]}" feature/search
git checkout -q -b fix/cache-race
commit "${ada[@]}" "Fix race in cache invalidation"
git checkout -q main
commit "${leo[@]}" "Polish empty states" ui/empty.rs
git checkout -q -b feature/billing
commit "${mira[@]}" "Stripe checkout flow"
commit "${mira[@]}" "Usage-based invoices"
git checkout -q main
commit "${ada[@]}" "Release notes for 1.1" CHANGELOG.md
git tag v1.1.0

git init -q --bare "$out/.remotes/aurora.git"
git remote add origin "$out/.remotes/aurora.git"
git push -q origin main fix/cache-race
git fetch -q origin
git checkout -q feature/billing
git worktree add -q "$out/aurora-hotfix" -b hotfix/login-loop main
echo "// TODO: proration" >> src/usagebased_invoices.rs
echo "STRIPE_KEY=" > .env.example

for name in nebula-api orbit-docs; do
  git init -q -b main "$out/$name"
  cd "$out/$name"
  git config commit.gpgsign false
  commit "${ada[@]}" "Initial commit" README.md
  commit "${leo[@]}" "First draft"
done

echo "$out"
