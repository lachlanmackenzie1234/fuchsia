---
name: review
description: >
  Performs an AI code review of Fuchsia changes (fuchsia.git and
  vendor/google) using the same Fuchsia review skills as the Gerrit CL Review
  Agent. Use when asked to review, self-review, or `/review` changes in a
  Fuchsia checkout. Self-contained: filters non-reviewable files up-front,
  runs one read-only reviewer per skill, and verifies findings via an
  adversarial challenger pass.
---

# Fuchsia code review

This skill reviews a Fuchsia change with the review skills listed in
`.agents/code-review/agent_configs.textproto` (fuchsia.git, plus the
vendor/google copy for vendor/google changes), the same files the Gerrit CL
Review Agent (`groupagent-fuchsia-review` / `groupagent-turquoise-review`)
reads. Local and hosted reviews therefore load the same skills.

It is self-contained: do not load or follow any other `review` skill. It is
named `review` so that `/review` in a Fuchsia checkout runs it.

## Overview & Flow

1.  **Scope & Up-Front Skip Globs**: Detects `fuchsia.git` and `vendor/google`
    changes (`jj` or `git`) and filters out non-reviewable files (`Cargo.lock`,
    `*.golden`, `*.pb.*`, binaries, `out/**`) before dispatching reviewers.
1.  **Parallel Round-1 Review**: Resolves domain skills from
    `.agents/code-review/agent_configs.textproto` via `resolve_skills.py` and
    runs one read-only `fuchsia_code_reviewer` per skill plus a `general`
    reviewer in parallel on the retained files.
1.  **Adversarial Challenger Pass (Round 2)**: Deduplicates Round-1 candidate
    findings and runs `fuchsia_code_challenger` (`references/challenger.md`) to
    score each finding 0-100, pruning false positives, pre-existing code, and
    findings below confidence `80`.
1.  **Merge & Render**: Writes verified findings and `skipped_files` to
    `review.json` and renders the Markdown report via `render_report.py`.

## Step 1: Determine the scope

1.  Check each repository in the checkout separately: the fuchsia.git root, and
    `vendor/google` inside it if that directory exists. Detect the VCS at each
    repository's root (`jj` if a `.jj` directory exists there, otherwise `git`).
1.  Pick the target, treating each repository with changes as its own scope:
    - If the user named revisions, commits, branches, or Gerrit changes, review
      those in their owning repository (or both repositories if both are named).
    - Otherwise, if either repository has working-copy changes (`jj diff -r @`
      or `git diff HEAD`), review every repository that does.
    - Otherwise, review the most recent commit (`@-` in `jj`, `HEAD` in `git`)
      in each repository ahead of `JIRI_HEAD` (or `origin/main`), falling back
      to the current directory's repository if neither is ahead.
1.  For each repository in scope, record its root, VCS, immutable commit
    hash(es) (or `working copy` for uncommitted changes), and changed files as
    paths relative to that repository, exactly as Gerrit shows them.

## Step 2: Summarize the intent

Read the commit description(s) and any task context from the conversation, and
write a short, factual intent summary: the objective and the key logic changes.
When reviewing code written earlier in the same session, do not include claims
about correctness or justifications for design choices; they bias the reviewers.

## Step 3: Filter skip globs and resolve review tasks

Run from the fuchsia.git root once per repository in scope:

```bash
fuchsia-vendored-python .agents/skills/review/scripts/resolve_skills.py \
  --files="<path 1>,<path 2>" \
  --project="<fuchsia|vendor/google>" \
  --output_format=envelope
```

If `fuchsia-vendored-python` is not on `PATH`, use
`prebuilt/third_party/python3/<host>/bin/python3` or any `python3` 3.9 or later;
both scripts use only the standard library.

- The output is a JSON object with `tasks` (`[{"skill_path": ..., "files":
  [...]}]`), `retained_files`, and `skipped_files`. Files matching default skip
  globs (`Cargo.lock`, lockfiles, `*.golden`, `*.snap`, `*.pb.*`, images,
  archives, `out/**`, `.venvs/**`) are excluded up-front and listed in
  `skipped_files`. Pass `--skip_globs="<glob 1>,<glob 2>"` if the user asked to
  skip extra paths, or `--no_default_skips` if the user explicitly asked to
  review skipped files.
- Keep tasks from different repositories separate so each keeps its own
  repository root and VCS.
- Pass `--project=fuchsia` with fuchsia.git-relative paths for fuchsia.git, and
  `--project=vendor/google` with `vendor/google`-relative paths for
  `vendor/google`. For `vendor/google`, the script also reads
  `vendor/google/.agents/code-review/agent_configs.textproto` and resolves its
  skills relative to `vendor/google`. Pass `--repo_root=<fuchsia.git root>` if
  you run the script outside the checkout.
- For each repository whose `retained_files` is non-empty, add one `general`
  task with no skill and `retained_files` (never `skipped_files`). It stands in
  for the hosted agent's default reviewer and covers correctness, error
  handling, edge cases, tests, and the Fuchsia style of the surrounding code. If
  `retained_files` is empty across all repositories in scope, skip Step 4 and
  render `decision: "not_applicable"` with `skipped_files` in Step 5.

## Step 4: Run the reviewers and adversarial challenger

Use `${TMPDIR:-/tmp}/fuchsia_review_<run_id>` as `<out_dir>`, where `<run_id>`
comes from `uuidgen`. Never write review files inside the checkout.

If `define_subagent` and `invoke_subagent` are available:

1.  **Round 1 (Reviewers)**: Define a subagent named `fuchsia_code_reviewer`
    with write tools enabled (so it can run VCS diff commands and write its
    result file). Its system prompt must say that it is a strictly read-only
    reviewer: it never edits files in the checkout and never runs builds or
    tests. It must read and follow
    `.agents/skills/review/references/reviewer.md`.
1.  In a single turn, invoke one `fuchsia_code_reviewer` per task (across all
    repositories in scope) so they run in parallel. Give each one: its
    repository root and VCS, that repository's target scope and commit hash(es),
    the intent summary, its task (`skill_path` and `files`, or `general` with
    `retained_files`), and its output file `<out_dir>/task_<n>.json`.
1.  Wait for all of them. If a reviewer fails or its output file is missing or
    invalid, invoke it once more; if it fails again, leave it out and say so in
    the report summary.
1.  **Deduplicate candidate findings**: Read all Round-1 result files per
    repository. Treat findings as duplicates when they have the same
    `file_path`, overlapping lines, and the same defect (file-level findings
    with no location: same `file_path` and defect). Keep the one from a skill
    task over `general`; between skills, keep the one whose `rule_reference`
    cites the rule that is violated; otherwise keep the more detailed comment.
    Never merge different defects at the same line. If no candidate findings
    remain, skip Round 2 and proceed to Step 5.
1.  **Round 2 (Adversarial Challenger Pass)**: For each repository with
    candidate findings, write `<out_dir>/draft_findings_<repo>.json` with a
    0-based `finding_index` on each candidate finding. Define a **separate**
    subagent named `fuchsia_code_challenger` with write tools enabled (strictly
    read-only on the checkout; never runs builds or tests) that must read and
    follow `.agents/skills/review/references/challenger.md`. Invoke one
    `fuchsia_code_challenger` per repository in parallel with its repository
    root, VCS, target scope and commit hash(es), intent summary,
    `<out_dir>/draft_findings_<repo>.json`, and output file
    `<out_dir>/evaluations_<repo>.json`.
1.  Join `<out_dir>/evaluations_<repo>.json` back onto the candidate findings by
    `finding_index`: keep only findings where `verdict == "accept"` and
    `confidence >= 80`, copy `confidence` onto each surviving finding, and apply
    `adjusted_severity` when set to `blocking` or `suggestion`.

Otherwise, follow `references/reviewer.md` yourself once per task in sequence,
deduplicate candidate findings, and then follow `references/challenger.md` to
challenge and score each candidate finding (`confidence` 0-100, keeping only
`verdict == "accept"` and `confidence >= 80`). Reviewing your own change in the
same session is not independent; say so in the report summary.

## Step 5: Merge, render, and present

1.  Combine verified results across all repositories in scope. For
    `vendor/google`, prefix every `file_path` in `reviewed_files`,
    `skipped_files`, and `findings` with `vendor/google/` (if not already
    prefixed) so all paths are relative to the fuchsia.git root. Write one
    merged JSON object to `<out_dir>/review.json` with:
    - `decision`: `changes_requested` if any surviving finding is `blocking`,
      `approved_with_suggestions` if there are only suggestions, `approved` if
      there are none, or `not_applicable` if there were no tasks
      (`retained_files` was empty).
    - `summary`: one paragraph summarizing the verified findings (and noting how
      many draft findings the challenger pruned, plus any reviewer that failed
      twice).
    - `scope`: the reviewed target per repository in scope (name both when the
      review spans both), for example `fuchsia.git` commit `1a2b3c4d` and
      `vendor/google` working copy (`git diff HEAD`).
    - `agent_skills`: the union of all reviewers' `agent_skills`.
    - `reviewed_files`: the union of all reviewers' `reviewed_files` (with
      `vendor/google/` prefixes applied).
    - `skipped_files`: all files matched by up-front skip globs (with
      `vendor/google/` prefixes applied).
    - `findings`: all surviving findings (with `vendor/google/` prefixes and
      `confidence` scores applied).
1.  Render the report with `--workspace` set to the fuchsia.git root and
    `--min_confidence=80`:

    ```bash
    fuchsia-vendored-python .agents/skills/review/scripts/render_report.py \
      --review_result_file=<out_dir>/review.json \
      --workspace=<fuchsia.git root> \
      --min_confidence=80
    ```

    If it reports a validation error, fix `review.json` and run it again.

1.  Present the rendered Markdown to the user unchanged. Findings are numbered
    in the report (`#1`, `#2`, ...); use those numbers when discussing fixes.
1.  If the user asked to fix the findings, apply the fixes in each repository
    with its VCS after presenting the report.

## Notes

- `agent_configs.textproto` matches like the hosted agent: `path_regex` must
  match the whole repo-relative path, a file matching any `exclude_filters`
  block is skipped, and an entry with no `project` applies to every project.
  Entries without `skills` add nothing here; the `general` task covers them.
- A skill without `/` resolves to `.agents/skills/<name>/SKILL.md`; a skill with
  `/` resolves to `<fuchsia.git root>/<skill>/SKILL.md`. A skill that already
  ends in `/SKILL.md` names the file itself. Skills from the vendor/google
  config resolve the same way relative to `vendor/google`. The script warns on
  stderr if a resolved `SKILL.md` is missing.
