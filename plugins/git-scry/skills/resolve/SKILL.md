---
name: resolve
description: Resolve the conflicts of the merge, rebase, cherry-pick or revert in progress with Scry's tools, each with a reason, for the user to review in Scry. Use when the user asks to resolve git conflicts.
argument-hint: "[path]"
allowed-tools: mcp__plugin_git-scry_scry__overview mcp__plugin_git-scry_scry__conflicts mcp__plugin_git-scry_scry__conflict
---

Resolve the conflicts of the operation in progress with Scry's tools (the scry MCP server). When a path is given, only that file's: $ARGUMENTS

1. Call overview, then conflicts(path) for each file to resolve, then conflict(path, id) for each open conflict: it shows ours, base and theirs, the commits behind each side and the candidates.
2. Resolve each open conflict with resolve. Prefer pick, a candidate Scry offers (ours, theirs, ours-then-theirs, base...), when one keeps what both sides meant. Write text only when none does, and then say in dropped what of ours or theirs is not in it, or "nothing".
3. Give each a reason the user can check quickly: which intent of each side is kept and why, in 1 to 3 sentences, naming each side by its label from overview.
4. If resolve answers that the file does not parse, fix that conflict with resolve again. If it answers that a conflict is already resolved by someone else, leave it as it is and say so in your summary; use replace: true only when the user asks for it.
5. Do not edit the conflicted files yourself, and do not stage, commit, continue, skip or abort the operation: the user reviews each resolution in Scry first.
6. End with a short summary of what you resolved and what you were unsure about, and ask the user to review it with `git scry`.
