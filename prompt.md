You are an expert Rust developer helping merge four related personal CLI tools into one unified program called `recomplog`.

The four tools are:
- `repslog` — workout / reps / sets / FIT import tracker
- `bodylog` — body measurements + sleep logging
- `nutlog` — nutrition / food logging + reports
- `bodydashboard` — HTML report generator that pulls data from the above via CLI calls

**Goal:** Combine them into a single, cohesive `recomplog` binary that supports body recomposition tracking. The new tool should feel like a natural evolution of the existing ones (short lowercase name, local-first, CLI-first, heavily documented, LLM-agent friendly with strong `--json` support).

Key observations:
- There is significant code and documentation duplication across the repos.
- Current cross-tool integration is done via subprocess calls + JSON parsing (fragile).
- All are Rust + SQLite + similar project layout and agent-oriented design.

**Important requirements:**
- Incorporate and extend the practices described in the existing `AGENTS.md` files (especially around LLM/agent usage, structured output, and tool design).
- Adopt the `clippy.toml` and `rustfmt.toml` configurations from the repos, along with the guidelines in any `CODING_PRACTICES.md` files, for code quality and consistency from the start.

**Your task:**
Start the merge. You have full freedom to decide:
- Project structure (single crate, Cargo workspace, module layout, etc.)
- How to unify the CLI (subcommands, top-level commands, etc.)
- Data model and database strategy
- How to handle the dashboard/report generation
- Migration approach from the old separate databases
- Naming of internal modules and shared code
- Documentation and agent guidance updates

Preserve the spirit of the original tools: excellent docs, agent-first design, local-only operation, and practical fitness tracking.

Begin by exploring the current state of the four projects (including their `AGENTS.md`, `CODING_PRACTICES.md`, `clippy.toml`, and `rustfmt.toml` files), then propose (and start implementing) a clean first step toward the unified `recomplog` tool. Explain your reasoning and decisions as you go.


---

Code is in ../repslog ../bodylog ../bodydashboard ../nutlog/

Make sure to also incorporate import functions for the databases of the previous tools.
