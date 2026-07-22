import { createInterface } from "node:readline/promises";
import { Agent } from "./classes/Agent.js";

const HELP_TEXT = `Commands:
  /help            Show this help
  /clear           Clear the conversation context
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

        const loaded = agent.loadSkillByName(cmd);
        console.log(loaded ? `Loaded skill: ${cmd}` : `Unknown command or skill: /${cmd}`);
        continue;
      }

      const reply = await agent.handleUserMessage(line);
      console.log(reply);
    }
  } finally {
    rl.close();
  }
}

main();
