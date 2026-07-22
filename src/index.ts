import { createInterface } from "node:readline/promises";
import { Agent } from "./classes/Agent.js";

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

  const rl = createInterface({ input: process.stdin, output: process.stdout });
  console.log('Ready. Type a message, or "/help" for commands.');

  try {
    for (;;) {
      let answer: string;
      try {
        answer = await rl.question("> ");
      } catch {
        break; // stdin closed (EOF / Ctrl+D) — exit cleanly instead of throwing
      }
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

        const loaded = agent.loadSkillByName(cmd);
        console.log(loaded ? `Loaded skill: ${cmd}` : `Unknown command or skill: /${cmd}`);
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
    rl.close();
  }
}

await main();
