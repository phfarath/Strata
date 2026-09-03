# Contributing to Strata

First off, thank you for considering contributing to **Strata**! 🎉

Strata is an open-source, local-first persistent memory engine and cognitive runtime for AI coding agents (Cursor, Claude Code, Codex CLI, Gemini CLI, Antigravity). We welcome contributions from developers of all backgrounds.

---

## 🧭 Engineering Philosophy

Before writing code, please keep these core tenets in mind:
1. **Radical Simplicity**: Write lean, straightforward Rust with well-designed types before introducing extra abstraction layers or boilerplate.
2. **Offline-First & Local Privacy**: Code and memory should run entirely on the developer's machine (`~/.strata/`) without unneeded cloud dependencies.
3. **Deterministic & Verifiable**: Strata uses cognitive science (ACT-R activation, Ebbinghaus retention curves, JTMS v2 truth maintenance). Changes must be backed by tests and verifiable logic.
4. **Strict TDD (Test-Driven Development)**: Every bugfix or new capability must come accompanied by unit tests or an evaluation scenario.

---

## 🏛️ Repository Architecture

Strata is organized as a Cargo workspace with strict module boundaries:

| Crate | Responsibility |
| :--- | :--- |
| **`crates/strata-core`** | Fundamental types, traits, schemas (Episodic, Semantic, Procedural, CDC), Merkle tree models. |
| **`crates/strata-memory`** | Local SQLite storage, FTS5 BM25 search, FastEmbed ONNX local embeddings, ACT-R decay engine. |
| **`crates/strata-tools`** | AST Tree-Sitter parsing, native call graph & import analyzer, monorepo workspace isolators. |
| **`crates/strata-reasoning`** | JTMS v2 deterministic conflict resolver, causal graph, cognitive tiering. |
| **`crates/strata-cli`** | The `strata` terminal binary, daemon, and local MCP server for IDEs. |
| **`crates/strata-evals`** | Deterministic evaluation suite and accuracy benchmarks. |

---

## 🛠️ Getting Started

### Prerequisites
- **Rust toolchain** (stable, 2021 edition or newer): [Install Rust](https://rustup.rs/)
- **C/C++ compiler toolchain** (required for compiling Tree-Sitter parsers and SQLite bindings):
  - **Linux**: `build-essential`, `clang`, `pkg-config`, `libssl-dev`
  - **macOS**: Xcode Command Line Tools (`xcode-select --install`)
  - **Windows**: Visual Studio Build Tools (C++ workload)

### Local Setup
```bash
# 1. Clone your fork
git clone https://github.com/<your-username>/Strata.git
cd Strata

# 2. Verify compilation
cargo check --workspace

# 3. Run all unit and integration tests
cargo test --workspace
```

---

## 🔄 Development Workflow

### 1. Branching Strategy
Direct pushes to `main` are disabled. All changes must go through a branch and a Pull Request.

- Name your branch with a clear prefix:
  - `feat/<feature-name>` for new features
  - `fix/<bug-name>` for bug fixes
  - `refactor/<module-name>` for refactoring without behavior change
  - `docs/<topic>` for documentation improvements
  - `test/<scenario>` for test suite additions

### 2. Conventional Commits
We follow the [Conventional Commits](https://www.conventionalcommits.org/) specification:
- `feat(memory): add support for custom embedding dimensions`
- `fix(cli): handle graceful shutdown on SIGINT in mcp server`
- `test(evals): add scenario for cross-package symbol resolution`
- `docs(readme): add installation guide for macOS Homebrew`

### 3. Code Style & Quality Checks
Before opening a Pull Request, ensure your code passes standard formatting and lints:

```bash
# Check formatting
cargo fmt --all -- --check

# Run Clippy lints
cargo clippy --workspace --all-targets -- -D warnings

# Run all test suites
cargo test --workspace
```

---

## 📋 Pull Request Checklist

When submitting a Pull Request, please ensure:
- [ ] Your code compiles cleanly without warnings (`cargo check --workspace`).
- [ ] All tests pass (`cargo test --workspace`).
- [ ] New logic is covered by unit tests or eval scenarios in `crates/strata-evals`.
- [ ] `cargo fmt` has been executed across the workspace.
- [ ] The PR description clearly explains the **motivation**, **approach**, and provides **steps to verify**.

---

## 🛡️ Security Disclosures

If you discover a security vulnerability within Strata, please do **not** open a public issue. Instead, report it privately via [GitHub Security Advisories](https://github.com/phfarath/Strata/security/advisories/new) or directly to the repository maintainers.

---

## 🤝 Community & Code of Conduct

We are committed to providing a friendly, welcoming, and inclusive environment for everyone. Please be respectful, constructive in reviews, and considerate of fellow contributors.
