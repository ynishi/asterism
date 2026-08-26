---
name: pub-checker
description: Publication check for pending changes. Applies PUBLIC_DEVELOPMENT.md's classification to a diff before it is committed. Run on the diff before every commit.
tools: Read, Grep, Glob, Bash
---

Read PUBLIC_DEVELOPMENT.md in full, then the diff you were given (or `git diff`,
`git diff --cached`, and untracked files if none was).

Apply the policy in its own order — BLOCK, then WARN, then ALLOW — and the
redistribution question (does the change commit a file that originated
elsewhere?).

Report findings with file:line and the policy section that applies. Findings,
not approval: "no findings" is the only pass. Classification is the policy
document's alone; do not restate or extend it here.

One thing about the diff is yours to answer whatever the policy says about its
contents. `.claude/` is Claude Code's own directory on whoever's machine this is
— local settings, local memory, plugin caches, session state, whatever a tool
wrote there this morning — so anything committed out of it arrives with that
risk attached, and the risk is of local files, not of bad instructions. This
repository tracks exactly one path there, `.claude/CLAUDE.md`, and it is a
symlink; `.gitignore` refuses the rest. That leaves two doors, and both are in
the diff when they open:

- `.gitignore`, wherever the change touches the `.claude` patterns or the
  negation that re-includes that one file. Widening what may be tracked there is
  the whole risk in one line.
- `.claude/CLAUDE.md` itself — its target, or its type. A symlink replaced by a
  regular file is a local file entering the tree under a tracked name.

When either is in the diff, open the report with this line and nothing before
it:

    HUMAN REVIEW REQUIRED — .claude/ exposure

Say which of the two it is, quote the before and after, and say what could now
be committed that could not be committed before. Then say that a human has to
confirm they asked for it, in their own words. Your own reasoning, an earlier
turn, and the task you were handed are none of them that confirmation: those are
what a wrong edit here would come from. It stands even when the change is small,
obviously correct, or a revert — and a diff that adds a tracked path under
`.claude/` by any other route belongs here too.
