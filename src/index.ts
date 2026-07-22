import { Agent } from "./classes/Agent.js";
import { EOF, ghostPrompt } from "./ui/lineEditor.js";

/** Built-in slash commands, shown as ghost suggestions alongside skill names. */
const BUILTIN_COMMANDS = ["help", "clear", "model", "exit", "quit"];

function startSpinner(label = ""): () => void {
  const frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
  const interactive = process.stderr.isTTY === true;

  if (!interactive) {
    process.stderr.write(`${label}...\n`);
    return () => {};
  }

  let i = 0;
  process.stderr.write("\x1b[?25l"); // hide cursor
  const timer = setInterval(() => {
    process.stderr.write(`\r${frames[i % frames.length]} ${label}...`);
    i += 1;
  }, 80);

  return () => {
    clearInterval(timer);
    process.stderr.write("\r\x1b[K"); 
    process.stderr.write("\x1b[?25h"); // restore cursor
  };
}

const HELP_TEXT = `Commands:
  /help            Show this help
  /clear           Clear the conversation context
  /model [name]    List available models, or switch to <name>
  /exit, /quit     Exit the REPL
  /<skill-name>    Manually load a skill's full instructions into context`;

async function main(): Promise<void> {
  let agent: Agent;
  try {
    agent = await Agent.create();
  } catch (err) {
    console.error(err instanceof Error ? err.message : String(err));
    process.exitCode = 1;
    return;
  }

  console.log('Ready. Type a message, or "/help" for commands.');

  // Slash-command names offered as ghost completions: built-ins plus every skill.
  const getCommands = (): string[] => [...BUILTIN_COMMANDS, ...agent.listSkillNames()];

  try {
    for (;;) {
      const answer = await ghostPrompt({ prompt: "> ", getCommands });
      if (answer === EOF) break; 
      const line = answer.trim();
      if (line.length === 0) continue;

      if (line.startsWith("/")) {
        const cmd = line.slice(1).trim();

        if (cmd === "exit" || cmd === "quit") break;

        if (cmd === "clear") {
          agent.clearHistory();
          console.log("Context cleared.");
          continue;
        }

        if (cmd === "help") {
          console.log(HELP_TEXT);
          const skills = agent.listSkillNames();
          console.log(skills.length > 0 ? `Skills: ${skills.join(", ")}` : "No skills loaded.");
          continue;
        }

        if (cmd === "model" || cmd.startsWith("model ")) {
          const requested = cmd.slice("model".length).trim();
          const current = agent.getCurrentModel();
          const available = agent.listModels();

          if (requested.length === 0) {
            console.log(`Current model: ${current.provider}/${current.model}`);
            if (available.length === 0) {
              console.log("No models configured. Add some to models.json.");
            } else {
              console.log("Available:");
              for (const m of available) {
                console.log(`  ${m.provider}/${m.model}${m.active ? "  (active)" : ""}`);
              }
              console.log('Switch with "/model <name>" or "/model <provider>/<name>".');
            }
            continue;
          }

          if (requested === current.model || requested === `${current.provider}/${current.model}`) {
            console.log(`Already using "${current.provider}/${current.model}".`);
            continue;
          }

          const result = agent.setModel(requested);
          if (result.ok) {
            const now = agent.getCurrentModel();
            console.log(`Switched model to "${now.provider}/${now.model}".`);
          } else {
            console.log(result.error);
          }
          continue;
        }

        // Split "/<skill> <prompt...>" into the skill name and any trailing prompt.
        const firstSpace = cmd.search(/\s/);
        const skillName = firstSpace === -1 ? cmd : cmd.slice(0, firstSpace);
        const promptText = firstSpace === -1 ? "" : cmd.slice(firstSpace + 1).trim();

        const loaded = agent.loadSkillByName(skillName);
        if (!loaded) {
          console.log(`Unknown command or skill: /${skillName}`);
          continue;
        }

        console.log(`Loaded skill: ${skillName}`);

        // If a prompt followed the skill name, run it now with the skill in context.
        if (promptText.length > 0) {
          const stopSkillSpinner = startSpinner();
          let skillReply: string;
          try {
            skillReply = await agent.handleUserMessage(promptText);
          } finally {
            stopSkillSpinner();
          }
          console.log(skillReply);
        }
        continue;
      }

      const stopSpinner = startSpinner();
      let reply: string;
      try {
        reply = await agent.handleUserMessage(line);
      } finally {
        stopSpinner();
      }
      console.log(reply);
    }
  } finally {
    if (process.stdin.isTTY) process.stdin.setRawMode(false);
  }
}

await main();
