# Strata Guard GitHub Action

> Automated Pull Request architectural review powered by **Strata Cognitive Memory, Causal World Model, and Failure Anti-Patterns**.

Strata Guard analyzes changed files in pull requests to evaluate breaking changes, architectural ripple risks, contract invariants, and relevant anti-patterns before code is merged.

---

## 🚀 Features

- **🎯 Causal Blast Radius & Risk Assessment**: Resolves direct and transitive dependencies touched by the PR, computing an architectural risk score (`Low`, `Moderate`, `Elevated`, `Critical`).
- **🧠 Contract Invariant Checking**: Enforces active architectural rules and system invariants anchored in code.
- **⚠️ Cognitive Failure Anti-Pattern Search**: Queries Strata's persistent episodic/semantic failure memory using the PR title and touched modules, surfacing concrete mitigations.
- **📌 Sticky PR Comments & Job Summary**: Automatically creates or updates a clean PR sticky comment (`<!-- strata-guard-report -->`) and writes the analysis report to `$GITHUB_STEP_SUMMARY`.
- **🛡️ Fork-Safe & Permission-Resilient**: Safely handles PRs from forks where write tokens are restricted.

---

## 📦 Usage

### Minimal Workflow Example

Add the following to `.github/workflows/strata-guard.yml`:

```yaml
name: Strata Guard

on:
  pull_request:
    types: [opened, synchronize, reopened]

concurrency:
  group: strata-guard-${{ github.workflow }}-${{ github.head_ref || github.ref }}
  cancel-in-progress: true

permissions:
  contents: read
  pull-requests: write
  issues: write

jobs:
  guard:
    name: Strata Causal PR Review
    runs-on: ubuntu-latest
    steps:
      - name: Checkout repository
        uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - name: Run Strata Guard Reviewer
        uses: ./.github/actions/strata-guard
        with:
          github-token: ${{ secrets.GITHUB_TOKEN }}
          pr-title: ${{ github.event.pull_request.title }}
          base-ref: ${{ github.base_ref }}
          comment-on-pr: 'true'
          fail-on-critical: 'false'
```

---

## ⚙️ Inputs

| Input | Description | Required | Default |
| :--- | :--- | :---: | :--- |
| `github-token` | GitHub token for posting PR comments and reading issues | No | `${{ github.token }}` |
| `pr-title` | Pull request title for querying memory and failure signatures | No | `${{ github.event.pull_request.title }}` |
| `base-ref` | Base branch ref for git diff | No | `${{ github.base_ref }}` |
| `comment-on-pr` | Whether to post or update a sticky PR review comment | No | `'true'` |
| `fail-on-critical`| Whether to fail the workflow step if risk is Critical | No | `'false'` |
| `strata-bin` | Custom path to pre-built strata binary (skips cargo build) | No | `''` |

---

## 📤 Outputs

| Output | Description |
| :--- | :--- |
| `risk-level` | Overall risk level (`Low`, `Moderate`, `Elevated`, `Critical`) |
| `risk-score` | Pre-code risk score percentage integer (`0` to `100`) |
| `safe-to-apply` | Boolean string (`true` or `false`) indicating if safe to merge |
| `report-markdown` | Full Markdown review text |

---

## 📄 Example Review Output

```markdown
## 🛡️ Strata Guard — Cognitive Architecture & PR Review

> Automated architectural risk assessment powered by Strata Cognitive Memory & Causal World Model.

### 🎯 Causal Blast Radius & Risk Level

| Metric | Value | Status |
| :--- | :--- | :--- |
| **Pre-Code Risk Score** | `42%` | 🟡 **MODERATE RISK** |
| **Risk Classification** | **Moderate** | 🟡 |
| **Modified Targets** | `1` files | 📁 |
| **Impacted Architectural Nodes** | `1` nodes | 🌐 |
| **Breaking Change Risks** | `1` detected | ⚠️ Warning |

### 🧠 Invariants & High-Importance Architectural Rules Checked
✅ All high-importance architectural contracts and constraints remain intact.

### ⚠️ Known Failure Anti-Patterns & Mitigations
✅ No known failure anti-patterns detected.

### 💡 Recommended Action
#### 🟢 Verdict: PASS
Changes are within safe architectural thresholds.
```
