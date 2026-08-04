---
name: create-skill
description: Create or update reusable Flowmation skills, including their SKILL.md instructions and optional scripts, references, or assets. Use when a user asks to teach the application a repeatable procedure, add domain guidance, package reusable instructions, improve an existing skill, or turn a recurring task into a skill.
---

# Create Skill

Create focused, reusable instructions for another agent. Prefer the smallest skill that reliably covers the user's examples.

## Understand the request

1. Identify two or three concrete prompts that should activate the skill.
2. Determine the expected output, required inputs, available tools, and important failure cases.
3. Ask only for choices that materially change the result. Otherwise, state reasonable assumptions and proceed.
4. Inspect an existing same-named skill before changing it and preserve useful project-specific behavior.

## Choose the location and name

- Default to `<project>/.work-agent/skills/<skill-name>/SKILL.md` so the skill travels with the project.
- Use `~/.work-agent/skills/<skill-name>/SKILL.md` only when the user explicitly wants the skill available across projects.
- Normalize the name to lowercase kebab-case. The directory name and frontmatter `name` must match.
- Never overwrite an existing skill without reading it first.

## Plan reusable contents

Keep the instructions in `SKILL.md`. Add only resources that will be reused:

- `scripts/` for deterministic operations that would otherwise be rewritten.
- `references/` for schemas, policies, or detailed guidance loaded only when needed.
- `assets/` for templates or files copied into future outputs.

Avoid duplicate guidance and auxiliary documentation such as a skill README or changelog. Keep references one level from `SKILL.md` and link each one directly from it.

## Write SKILL.md

Use only `name` and `description` in frontmatter unless Flowmation configuration metadata is genuinely required:

```markdown
---
name: example-skill
description: Perform a specific reusable task. Use when the user asks for concrete trigger examples or related work.
---

# Example Skill

Follow concise, imperative instructions here.
```

Make the description the complete trigger contract: say what the skill does and when it applies. Write the body in imperative form and include only knowledge another capable agent would not reliably infer.

For configurable text, declare defaults under `metadata.flowmation.config` and use `${VARIABLE_NAME}` placeholders in the body. Project values in `.work-agent/config.json` override global values. Do not place secret values in skill files.

## Validate and finish

1. Read every created or changed file back.
2. Confirm the frontmatter delimiters parse, required fields are non-empty, and the name matches the directory.
3. Search for leftover placeholders, duplicated sections, invalid links, and references to missing resources.
4. Run any bundled script on a representative safe input.
5. Summarize the files created, the prompts that trigger the skill, and any assumptions.
6. Tell the user to restart Flowmation before invoking a newly created skill with `/<skill-name> [message]`; skill discovery occurs when the application starts.
