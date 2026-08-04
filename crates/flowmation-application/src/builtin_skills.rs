pub(crate) struct BuiltinSkillSource {
    pub name: &'static str,
    pub raw: &'static str,
}

pub(crate) const BUILTIN_SKILLS: &[BuiltinSkillSource] = &[
    BuiltinSkillSource {
        name: "create-schedule",
        raw: include_str!("../builtin-skills/create-schedule/SKILL.md"),
    },
    BuiltinSkillSource {
        name: "create-skill",
        raw: include_str!("../builtin-skills/create-skill/SKILL.md"),
    },
    BuiltinSkillSource {
        name: "create-workflow",
        raw: include_str!("../builtin-skills/create-workflow/SKILL.md"),
    },
];
