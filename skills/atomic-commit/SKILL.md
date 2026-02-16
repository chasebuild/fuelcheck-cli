---
name: atomic-commit
description: Split changes into atomic commits with clean staging and concise messages. Use when the user asks for "atomic commits", "split this into commits", "commit hygiene", or wants guidance on separating changes into reviewable pieces.
---

# Atomic Commit

Use this skill to split a working tree into small, logical commits that are easy to review.

## Read First

- If repo-specific PR or commit workflow docs exist, follow them.
- For OpenClaw-style guidance, see `references/openclaw-pr-workflow.md`.

## Workflow

1. Inventory changes.
- Run `git status -sb` and `git diff`.
- Group changes by intent (feature, fix, refactor, formatting, docs, tests).

2. Split by intent.
- Separate behavior changes from formatting.
- Split refactors from functional changes when possible.
- Keep tests with the change they validate.

3. Stage atomically.
- Use `git add -p` to stage only the hunks for the current commit.
- Use `git add -N` to partially stage new files.
- Re-check with `git diff --staged` before committing.

4. Commit message guidance.
- Keep messages concise, present-tense, action-oriented.
- Use a small scope prefix when helpful: `<area>: <action>`.

5. Verify cleanliness.
- After each commit, ensure the staged diff is empty and remaining changes match the next commit group.

6. Ask before risky splits.
- If a change is tightly coupled or splitting would create broken intermediate states, ask before proceeding.

## Output

After splitting, report:
- Commit list (hash + message)
- What remains uncommitted

