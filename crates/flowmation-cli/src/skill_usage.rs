use flowmation_application::AgentActivity;
use serde_json::Value;

#[derive(Debug, Default)]
pub struct SkillUsageTracker {
    names: Vec<String>,
}

impl SkillUsageTracker {
    pub fn observe(&mut self, activity: AgentActivity) -> Option<String> {
        let AgentActivity::ToolCompleted {
            tool_name,
            arguments,
            succeeded,
        } = activity;
        if tool_name != "load_skill" || !succeeded {
            return None;
        }
        let name = arguments.get("name").and_then(Value::as_str)?;
        if self.names.iter().any(|used_name| used_name == name) {
            return None;
        }
        self.names.push(name.to_owned());
        Some(format!("Using skill: {name}"))
    }

    pub fn summary(&self) -> Option<String> {
        match self.names.as_slice() {
            [] => None,
            [name] => Some(format!("Used skill: {name}")),
            names => Some(format!("Used skills: {}", names.join(", "))),
        }
    }
}

#[cfg(test)]
mod tests {
    use flowmation_application::AgentActivity;
    use serde_json::{Map, Value};

    use super::SkillUsageTracker;

    fn completed_tool_activity(
        tool_name: &str,
        skill_name: Option<&str>,
        succeeded: bool,
    ) -> AgentActivity {
        let mut arguments = Map::new();
        if let Some(skill_name) = skill_name {
            arguments.insert("name".to_owned(), Value::String(skill_name.to_owned()));
        }
        AgentActivity::ToolCompleted {
            tool_name: tool_name.to_owned(),
            arguments,
            succeeded,
        }
    }

    #[test]
    fn reports_each_successfully_loaded_skill_once() {
        let mut usage = SkillUsageTracker::default();

        assert_eq!(
            usage.observe(completed_tool_activity("load_skill", Some("report"), true)),
            Some("Using skill: report".to_owned())
        );
        assert_eq!(
            usage.observe(completed_tool_activity("load_skill", Some("report"), true)),
            None
        );
        assert_eq!(
            usage.observe(completed_tool_activity(
                "load_skill",
                Some("finance/reconcile"),
                true,
            )),
            Some("Using skill: finance/reconcile".to_owned())
        );
        assert_eq!(
            usage.summary().as_deref(),
            Some("Used skills: report, finance/reconcile")
        );
    }

    #[test]
    fn ignores_failed_skill_loads_and_other_tools() {
        let mut usage = SkillUsageTracker::default();

        assert_eq!(
            usage.observe(completed_tool_activity(
                "load_skill",
                Some("missing"),
                false,
            )),
            None
        );
        assert_eq!(
            usage.observe(completed_tool_activity("read_file", Some("report"), true)),
            None
        );
        assert_eq!(
            usage.observe(completed_tool_activity("load_skill", None, true)),
            None
        );
        assert_eq!(usage.summary(), None);
    }
}
