---
name: noop-pruner
description: Prune generated output until every sentence changes behavior, constrains format, or preserves required information.
version: 1.0.0
---

# Noop Pruner
<!-- inspired by Matt Pocock (writing-for-agents), Strunk & White (Rule 17) -->

## When to Use

Use for prompts, responses, handoffs, rubrics, and workflow text.

## Procedure

1. Name the output job
   1.1 State what the reader or agent must do after reading it.

2. Treat soft prose as suspect
   2.1 Assume agent-written filler is guilty until it changes output.
   2.2 No-ops are harmful: they blur intent, hide rules, and waste tokens.

3. Run the removal test
   3.1 Remove one sentence mentally.
   3.2 If nothing changes, delete it.
   3.3 Rewrite only when it can become an observable constraint.

4. Keep one source of truth
   4.1 One meaning, one authoritative sentence.

5. Keep only live text
   5.1 Behavior change
   5.2 Exact format
   5.3 Scope boundary
   5.4 Required evidence
   5.5 Stop/loop condition
   5.6 Failure-preventing example

6. Preserve requested layout
   6.1 Match the user's shown format exactly.
   6.2 Use sub-numbering only for real subtasks.

7. Return the controlled output
   7.1 Output the revised text only unless asked for notes.
