---
name: auto-git-commit
description: Automate git commits with correct staging and messages. Use when the user asks to "commit my changes", "auto commit", "stage and commit", "commit everything", or wants commits created on their behalf. Covers deciding whether to split changes, staging with git add -p, and producing concise commit messages.
---

# Auto Git Commit

Use this skill to create commits on the user's behalf while keeping changes scoped and reviewable.

## Read First

- If repo-specific PR or commit workflow docs exist, follow them.
- For OpenClaw-style guidance, see `references/openclaw-pr-workflow.md`.

## Workflow

1. Check status and diff.
- Run `git status -sb` and `git diff`.
- Identify logical change groups (features, refactors, formatting, docs, tests).

2. Decide commit grouping.
- If a single logical change, make one commit.
- If multiple concerns, split into multiple atomic commits.

3. Stage carefully.
- Prefer `git add -p` to stage only relevant hunks for the current commit.
- If needed, use `git add -N` then `git add -p` to split newly added files.
- Avoid staging unrelated files or generated artifacts unless explicitly requested.

4. Use repo tooling when present.
- If `scripts/committer` or a documented commit helper exists, use it.
- Otherwise, use `git commit -m "<area>: <action>"`.

5. Message guidance.
- Keep messages concise, present-tense, action-oriented.
- Prefer the smallest clear scope (e.g., `engine: fix cache key`).
- Do not include PR numbers unless the repo requires it.

6. Confirm before committing if:
- There are untracked files that look unrelated.
- The diff mixes formatting and behavior.
- You are unsure about splitting vs. single commit.

## Output

After committing, report:
- Commit hash
- Files included
- Any remaining unstaged changes

