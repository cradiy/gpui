---
name: canonical-documentation
description: Write or revise project documentation, READMEs, API guides, examples, usage text, and public code comments in any repository. Use whenever development work creates or changes such material. Keep documentation focused on the final current behavior and exclude the development conversation and implementation history.
---

# Canonical Documentation

Write documentation as the current source of truth for the project.

## Content boundary

- Include only the final public behavior, API, caller-relevant constraints, and
  concise usage.
- Never transfer the user conversation or development process into
  documentation. Exclude request history, feedback, rejected approaches,
  debugging causes, implementation iterations, and explanations of what was
  changed during the task.
- Keep development details in the task reply, commit message, review, or an
  explicitly requested changelog or migration guide.
- Preserve user-authored documentation and wording unless the requested change
  requires editing it.

## Writing style

- Describe the current state directly, as if the documented API had always
  existed.
- Do not frame headings, examples, or run instructions as patches, follow-up
  additions, or separations from earlier work.
- Name examples after the capability they demonstrate and state commands
  directly.
- Include rationale only when a reader needs it to choose or configure public
  behavior.
