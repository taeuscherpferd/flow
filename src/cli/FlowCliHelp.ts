export const BUILTIN_COMMANDS = [
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

export const HELP_TEXT = `Commands:
  /agent [name]    List agents or switch to a persistent specialist conversation
  /agent main      Return to the coordinator
  /clear           Clear only the active conversation
  /model [name]    List or switch the active conversation's model
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
  Up/Down          Browse input history from this and earlier sessions`;

export const READY_TEXT = 'Ready. Type a message, or "/help" for commands.';

export const WELCOME_TEXT =
  "Welcome to flowmation. Before we can get started you will need to setup a provider and a model. Use /model to get started.";
