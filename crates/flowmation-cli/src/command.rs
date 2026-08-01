#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplCommand {
    Empty,
    Message(String),
    Help,
    Agent(Option<String>),
    Clear,
    Model(Option<String>),
    Workflows,
    Workflow(String),
    Runs,
    WorkflowDebug(Option<String>),
    Run(String),
    Resume(String),
    Cancel(String),
    Schedules,
    Schedule(String),
    Dynamic(String),
    Exit,
}

pub const BUILTIN_COMMANDS: &[&str] = &[
    "help",
    "agent",
    "clear",
    "model",
    "workflows",
    "workflow",
    "runs",
    "workflow-debug",
    "run",
    "resume",
    "cancel",
    "schedules",
    "schedule",
    "exit",
    "quit",
];

pub const HELP_TEXT: &str = r#"Commands:
  /agent [name]    List agents or switch to a persistent specialist conversation
  /agent main      Return to the coordinator
  /clear           Clear only the active conversation
  /model [name]    List or switch the active conversation's model
  /model openai[/<name>]
                    Add and switch to an OpenAI model through ChatGPT
  /workflows       List workflows owned by the active agent
  /workflow <name> [input]
                    Run an active-agent workflow
  /runs            List recent project workflow runs
  /run <id>        Inspect a workflow run
  /resume <id>     Resume a waiting or interrupted workflow run
  /cancel <id>     Cancel a workflow run
  /schedules       List project schedules
  /schedule <id>   Inspect a schedule and its occurrences
  /schedule pause|resume|delete|reauthorize <id>
                    Manage a schedule
  /workflow-debug [on|off]
                    Show or hide workflow and agent status messages
  /<skill>         Load an active-agent skill
  /<agent>/<skill> Load a specialist skill from main
  /exit, /quit     Exit the REPL

Keyboard:
  Up/Down          Browse input history from this and earlier sessions"#;

#[must_use]
pub fn parse_repl_line(line: &str) -> ReplCommand {
    if line.is_empty() {
        return ReplCommand::Empty;
    }
    let Some(command) = line.strip_prefix('/') else {
        return ReplCommand::Message(line.to_owned());
    };
    let command = command.trim();
    let (name, remainder) = command
        .split_once(char::is_whitespace)
        .map_or((command, ""), |(name, remainder)| (name, remainder.trim()));
    match name {
        "exit" | "quit" if remainder.is_empty() => ReplCommand::Exit,
        "help" if remainder.is_empty() => ReplCommand::Help,
        "agent" => ReplCommand::Agent(nonempty(remainder)),
        "clear" if remainder.is_empty() => ReplCommand::Clear,
        "model" => ReplCommand::Model(nonempty(remainder)),
        "workflows" if remainder.is_empty() => ReplCommand::Workflows,
        "workflow" => ReplCommand::Workflow(remainder.to_owned()),
        "runs" if remainder.is_empty() => ReplCommand::Runs,
        "workflow-debug" => ReplCommand::WorkflowDebug(nonempty(remainder)),
        "run" => ReplCommand::Run(remainder.to_owned()),
        "resume" => ReplCommand::Resume(remainder.to_owned()),
        "cancel" => ReplCommand::Cancel(remainder.to_owned()),
        "schedules" if remainder.is_empty() => ReplCommand::Schedules,
        "schedule" => ReplCommand::Schedule(remainder.to_owned()),
        _ => ReplCommand::Dynamic(command.to_owned()),
    }
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{ReplCommand, parse_repl_line};

    #[test]
    fn routes_builtin_commands_and_aliases() {
        assert_eq!(parse_repl_line("/quit"), ReplCommand::Exit);
        assert_eq!(
            parse_repl_line("/model reviewer"),
            ReplCommand::Model(Some("reviewer".to_owned()))
        );
        assert_eq!(
            parse_repl_line("/schedule pause abc"),
            ReplCommand::Schedule("pause abc".to_owned())
        );
        assert_eq!(
            parse_repl_line("/custom value"),
            ReplCommand::Dynamic("custom value".to_owned())
        );
    }
}
