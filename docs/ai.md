# GitChrono AI — research & plan

_Researched 2026-10-23. Prices from Mistral's official pricing page; verify before launch._

## Business model

GitChrono Pro is **AI only**, two ways to pay:

1. **GitChrono AI credits** — prepaid, hosted by us, Mistral models, small margin.
2. **Bring your own key (BYOK)** — free. The app talks directly to the user's provider
   (OpenAI-compatible endpoints, Anthropic, Gemini, Mistral, OpenRouter, or a local
   Ollama / LM Studio). Nothing passes through our servers.

The core Git client stays free (Apache-2.0).

## Model choice: Mistral

| Model | Input $/M | Cached in $/M | Output $/M | Use for |
|---|---|---|---|---|
| Ministral 3 8B | 0.15 | 0.015 | 0.15 | fallback for very long outputs |
| **Mistral Small 4** | **0.15** | 0.015 | **0.60** | **default: every short-output feature** |
| **Mistral Large 3** | **0.50** | 0.05 | **1.50** | conflict resolution, commit composer |
| Codestral | 0.30 | 0.03 | 0.90 | — (built for completion, not prose) |
| Mistral Medium 3.5 | 1.50 | 0.15 | 7.50 | too expensive for this |

Batch API is 50% off (useful for release notes later, not interactive features).

**Why Mistral over the alternatives**

- Git features are **input-heavy** (a diff in, a few lines out), so input price dominates.
  Small 4's $0.15/M input is on par with the cheapest serious models
  (DeepSeek V4-Flash ~$0.14, Gemini Flash-Lite ~$0.10 per aggregators).
- **EU company** — "your diffs stay in Europe" is a real selling point for a Dutch product
  and for companies wary of sending source code to US/China providers.
  _Verify in Mistral's DPA that API data isn't used for training before claiming it on the site._
- Small 4 and Large 3 are Apache-2.0 open weights: if Mistral ever reprices, we can serve the
  same models elsewhere, and BYOK users can run them locally with identical prompts.
- DeepSeek is cheaper on output, but sending customer source code to a China-hosted API is
  a hard sell to companies; offer it via BYOK instead.

## Cost per action (Mistral, raw cost to us)

Assumptions: diffs are truncated to a token budget; outputs are short.

| Feature | Model | Tokens in / out | Raw cost | At 1.3× (user pays) |
|---|---|---|---|---|
| Commit message | Small 4 | 3k / 150 | $0.00054 | $0.0007 |
| Branch / stash name | Small 4 | 1.5k / 20 | $0.00024 | $0.0003 |
| Explain commit or diff | Small 4 | 4k / 400 | $0.00084 | $0.0011 |
| PR title + description | Small 4 | 15k / 600 | $0.0026 | $0.0034 |
| Release notes (tag range) | Small 4 | 12k / 1.5k | $0.0027 | $0.0035 |
| Conflict resolution (per file) | Large 3 | 6k / 1.5k | $0.0053 | $0.0068 |
| Commit composer (split WIP) | Large 3 | 12k / 1k | $0.0075 | $0.0098 |

**$10 of credits ≈ 14,000 commit messages** or ~9,000 commit explanations.

## Credit pricing: small margin, never at a loss

Show the balance in money (e.g. "$7.42 left"), not abstract credits, and charge
**actual token usage × 1.3**. The multiplier covers payment fees, failed or abandoned calls and a
thin margin:

| Pack | Stripe fee (~2.9% + $0.30) | Max raw inference if fully used | Our margin if fully used |
|---|---|---|---|
| $5 | $0.45 | $3.85 | $0.70 (14%) |
| $10 | $0.59 | $7.69 | $1.72 (17%) |
| $25 | $1.03 | $19.23 | $4.74 (19%) |

- Never at a loss: we meter every response's `usage` field and refuse requests when the balance
  can't cover a worst-case estimate (max input budget + max output tokens) up front.
- VAT goes on top at checkout (Stripe Tax). It's not margin. Alternatively a merchant-of-record
  (Paddle / Lemon Squeezy) handles EU VAT for ~5% + $0.50.
- Optional: a one-time **$0.50 free trial balance** for new accounts, about 700 commit messages.
- Pin the model version per feature (e.g. `mistral-small-4-…`), so a silent model swap can't
  change our costs.

For reference: GitKraken bundles ~250k tokens/week with paid plans on Gemini Flash; we're
pay-as-you-go with no subscription.

## Status

**Shipped (BYOK):** crate `gitchrono-ai`.
- Providers: Mistral, OpenAI, Anthropic, Gemini, OpenRouter and Ollama, plus any OpenAI-compatible endpoint via a custom URL.
- Commit message ("✨ Write message" in the staging panel), branch name and stash message ("✨ Suggest" in those prompts).
- Keys go in the OS keychain, with env-var fallback. Settings live in `<config dir>/gitchrono/ai.json`.
- Token usage of the last request is shown under the commit box.
- Try it from the CLI: `cargo run -p gitchrono-ai --example commit_message -- <repo>`.

**Next:** explain commit, then the hosted credits provider once the Kumavolt backend exists.

## Features, prioritized

### Phase 1 — cheap, high value
1. **Commit message** from the staged diff, a button next to the summary field. It learns the repo's
   style from the last ~50 subjects (Conventional Commits, tense, ticket prefixes) and streams into the inputs.
2. **Branch name** suggestion when creating a branch from WIP.
3. **Stash message** from the working changes.
4. **Explain** a commit, a diff, or "what's on this branch vs main", in the detail panel.

### Phase 2 — differentiators
5. **Commit composer**: split a messy WIP into logical commits, each with a message. We already
   have line/hunk patch application (`partial_patch`), so the model only has to propose groupings and we apply them.
6. **Release notes** for a tag range, with optional publish to the gitchrono.com changelog. This ties in the
   original GitChrono SaaS idea.
7. **PR title + description** (needs the hosting integrations).
8. **Conflict resolution**: suggest a resolution per conflicted hunk, shown in the 3-way editor for review.

### Phase 3 — later
9. **Natural-language commands** in the palette ("undo my last commit but keep the changes"):
   show the exact git command plus a preview, and run it only on confirm.
10. **Pre-push review**: flag secrets, debug leftovers, or "this commit mixes two concerns".
11. Semantic history search (needs embeddings and an index, so cost and complexity are higher).

## Architecture

```
gitchrono-app ──► gitchrono-ai (new crate)
                    ├─ prompts/      per-feature prompt builders + token budgeting
                    ├─ Provider trait: stream(prompt) -> text chunks, usage
                    ├─ HostedProvider     → https://gitchrono.com/api/ai  (credits)
                    ├─ OpenAiCompatible   → OpenAI, Mistral, DeepSeek, Groq, OpenRouter, Ollama, LM Studio
                    ├─ AnthropicProvider
                    └─ GeminiProvider
```

- **Keys** live in the OS keychain (`keyring` crate), never in config files.
- **Token budgeting**: prioritise the most-changed files, drop lockfiles, generated code and
  binaries, and summarise large files by hunk headers.
- **Privacy**: a "what will be sent" preview, secret scrubbing before sending (`.env`, keys,
  tokens), a per-repo AI off switch, and no telemetry of prompt content.
- **Hosted backend**: lives in the Kumavolt Laravel platform. Cashier handles credit purchases,
  `laravel/ai` / Prism handles the Mistral calls, and a ledger table debits `usage × price × 1.3`.
  Per-account rate limits and a max request size block abuse.
- **App sign-in**: OAuth device flow against gitchrono.com (the user approves in the browser), and the app
  stores the token in the keychain.

## Open questions
- Mistral DPA: training opt-out and data retention for API traffic (needed for the privacy claim).
- EUR or USD pricing for EU customers.
- Whether to include a tiny free trial balance (abuse risk vs. conversion).
