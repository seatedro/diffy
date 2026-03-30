---
name: "yeet"
description: "Use only when the user explicitly asks to stage, commit, push, and open a GitHub pull request in one flow using the GitHub CLI (`gh`)."
---

## Naming conventions

- Branch: `{description}` when starting from main/master/default.
- Commit: `verb({description})` (terse).
- PR title: `verb({description})` summarizing the full diff.

## Workflow

- If on main/master/default, create a branch
- Otherwise stay on the current branch.
- Confirm status, then stage everything: `git status -sb` then `git add -A`.
- Commit tersely with the description: `git commit -m "{description}"`
- Push with tracking: `git push -u origin $(git branch --show-current)`
- If git push fails due to workflow auth errors, pull from master and retry the push.
- Open a PR and edit title/body to reflect the description and the deltas: `GH_PROMPT_DISABLED=1 GIT_TERMINAL_PROMPT=0 gh pr create --fill --head $(git branch --show-current)`
- Write the PR description to a temp file with real newlines (e.g. pr-body.md ... EOF) and run pr-body.md to avoid \\n-escaped markdown.
- PR description (markdown) must be concise prose covering the issue, the cause and effect on users, the root cause, the fix, and any tests or checks used to validate.
