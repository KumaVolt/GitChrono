#!/usr/bin/env bash
# Builds the demo repositories used for the website and README screenshots:
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

commit() { # author email message [file]   (a file the caller already wrote is committed as is)
  local file=${4:-src/$(echo "$3" | tr 'A-Z ' 'a-z_' | tr -cd 'a-z_').rs}
  mkdir -p "$(dirname "$file")"
  [ -n "$(git status --porcelain -- "$file")" ] || echo "// $3" >> "$file"
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
# Real code for the two diffs the README screenshots show.
cat > src/stripe_checkout_flow.rs <<'RS'
//! Stripe checkout: turns a cart into a hosted checkout session.

use crate::billing::{Cart, Customer, Money};
use crate::config::Config;

pub struct CheckoutSession {
    pub id: String,
    pub url: String,
    pub expires_at: i64,
}

pub async fn create_session(
    config: &Config,
    customer: &Customer,
    cart: &Cart,
) -> anyhow::Result<CheckoutSession> {
    let total: Money = cart.items.iter().map(|item| item.price * item.quantity).sum();
    anyhow::ensure!(total.cents > 0, "cart is empty");

    let session = config
        .stripe()
        .checkout_sessions()
        .create(&customer.stripe_id, &cart.items, &config.success_url, &config.cancel_url)
        .await?;

    tracing::info!(customer = %customer.id, total = %total, "checkout session created");
    Ok(CheckoutSession { id: session.id, url: session.url, expires_at: session.expires_at })
}
RS
commit "${mira[@]}" "Stripe checkout flow"
cat > src/usagebased_invoices.rs <<'RS'
//! Monthly invoices for metered usage.

use crate::billing::{Customer, Money, Plan, UsageRecord};

pub struct Invoice {
    pub customer_id: String,
    pub lines: Vec<(String, Money)>,
    pub total: Money,
}

pub fn invoice(
    customer: &Customer,
    plan: &Plan,
    usage: &[UsageRecord],
) -> Invoice {
    let mut lines = vec![(plan.name.clone(), plan.base_price)];
    let units: u64 = usage.iter().map(|r| r.units).sum();
    let billable = units.saturating_sub(plan.included_units);
    if billable > 0 {
        lines.push((format!("{billable} extra units"), plan.unit_price * billable));
    }
    let total = lines.iter().map(|(_, amount)| *amount).sum();
    Invoice { customer_id: customer.id.clone(), lines, total }
}
RS
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
perl -0pi -e 's/(usage: &\[UsageRecord\],\n)/$1    days_active: u32,\n/; s/(    let mut lines = vec!\[\(plan.name.clone\(\), )plan.base_price/    \/\/ Customers who joined mid-month pay for the days they had.\n    let base = plan.base_price * days_active as u64 \/ 30;\n$1base/' src/usagebased_invoices.rs
echo "STRIPE_KEY=" > .env.example

for name in nebula-api orbit-docs; do
  git init -q -b main "$out/$name"
  cd "$out/$name"
  git config commit.gpgsign false
  commit "${ada[@]}" "Initial commit" README.md
  commit "${leo[@]}" "First draft"
done

echo "$out"
