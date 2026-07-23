import { Agent } from "../classes/Agent.js";
import { ConfigService } from "../services/ConfigService.js";
import { ModelSetupService } from "../services/ModelSetupService.js";
import { EOF, ghostPrompt } from "../ui/lineEditor.js";
import { startSpinner } from "../ui/spinner.js";

const BUILTIN_COMMANDS = ["help", "clear", "model", "exit", "quit"];

const HELP_TEXT = `Commands:
  /help            Show this help
  /clear           Clear the conversation context
  /model [name]    Set up the first model, list models, or switch to <name>
  /exit, /quit     Exit the REPL
  /<skill-name>    Manually load a skill's full instructions into context`;

const READY_TEXT = 'Ready. Type a message, or "/help" for commands.';

const WELCOME_TEXT =
  "Welcome to flowmation. Before we can get started you will need to setup a provider and a model. Use /model to get started.";

export class FlowCli {
  private agent: Agent | undefined;

  constructor(private readonly configService = new ConfigService()) {}

  async run(): Promise<void> {
    if (!(await this.initialize())) return;

    console.log(this.agent ? READY_TEXT : WELCOME_TEXT);

    try {
      await this.runPromptLoop();
    } finally {
      if (process.stdin.isTTY) process.stdin.setRawMode(false);
    }
  }

  private async initialize(): Promise<boolean> {
    try {
      const config = await this.configService.load();
      if (this.configService.hasConfiguredDefaultModel(config.models)) {
        this.agent = await Agent.create(this.configService);
      }
      return true;
    } catch (error) {
      console.error(error instanceof Error ? error.message : String(error));
      process.exitCode = 1;
      return false;
    }
  }

  private async runPromptLoop(): Promise<void> {
    for (;;) {
      const answer = await ghostPrompt({
        prompt: "> ",
        getCommands: () => this.getCommands(),
      });
      if (answer === EOF) return;

      const line = answer.trim();
      if (line.length === 0) continue;
      if (await this.handleLine(line)) return;
    }
  }

  private getCommands(): string[] {
    return [
      ...BUILTIN_COMMANDS,
      ...(this.agent?.listSkillNames() ?? []),
    ];
  }

  private async handleLine(line: string): Promise<boolean> {
    if (!line.startsWith("/")) {
      await this.respondToUser(line);
      return false;
    }

    return this.handleCommand(line.slice(1).trim());
  }

  private async handleCommand(command: string): Promise<boolean> {
    if (command === "exit" || command === "quit") return true;

    if (command === "clear") {
      this.clearHistory();
      return false;
    }

    if (command === "help") {
      this.showHelp();
      return false;
    }

    if (command === "model" || command.startsWith("model ")) {
      await this.handleModelCommand(command.slice("model".length).trim());
      return false;
    }

    await this.handleSkillCommand(command);
    return false;
  }

  private clearHistory(): void {
    const agent = this.getAgentOrShowWelcome();
    if (!agent) return;

    agent.clearHistory();
    console.log("Context cleared.");
  }

  private showHelp(): void {
    console.log(HELP_TEXT);
    const skills = this.agent?.listSkillNames() ?? [];
    console.log(
      skills.length > 0
        ? `Skills: ${skills.join(", ")}`
        : "No skills loaded.",
    );
  }

  private async handleModelCommand(requested: string): Promise<void> {
    if (!this.agent) {
      await this.setupFirstModel();
      return;
    }

    const current = this.agent.getCurrentModel();
    const available = this.agent.listModels();

    if (requested.length === 0) {
      console.log(`Current model: ${current.provider}/${current.model}`);
      console.log("Available:");
      for (const model of available) {
        const activeLabel = model.active ? "  (active)" : "";
        console.log(`  ${model.provider}/${model.model}${activeLabel}`);
      }
      console.log(
        'Switch with "/model <name>" or "/model <provider>/<name>".',
      );
      return;
    }

    const currentModel = `${current.provider}/${current.model}`;
    if (requested === current.model || requested === currentModel) {
      console.log(`Already using "${currentModel}".`);
      return;
    }

    const result = this.agent.setModel(requested);
    if (!result.ok) {
      console.log(result.error);
      return;
    }

    const active = this.agent.getCurrentModel();
    console.log(`Switched model to "${active.provider}/${active.model}".`);
  }

  private async setupFirstModel(): Promise<void> {
    const setupService = new ModelSetupService(
      this.configService,
      (prompt) => ghostPrompt({ prompt, getCommands: () => [] }),
      (message) => console.log(message),
    );
    const result = await setupService.run();

    if (result.status === "cancelled") {
      console.log("Setup cancelled. Use /model when you're ready.");
      return;
    }

    console.log(
      `Configured "${result.provider}/${result.model}" in ${result.configPath}.`,
    );

    try {
      this.agent = await Agent.create(this.configService);
      console.log(READY_TEXT);
    } catch (error) {
      console.error(
        `The model was saved, but the agent could not start: ${
          error instanceof Error ? error.message : String(error)
        }`,
      );
    }
  }

  private async handleSkillCommand(command: string): Promise<void> {
    const agent = this.getAgentOrShowWelcome();
    if (!agent) return;

    const firstSpace = command.search(/\s/);
    const skillName =
      firstSpace === -1 ? command : command.slice(0, firstSpace);
    const promptText =
      firstSpace === -1 ? "" : command.slice(firstSpace + 1).trim();

    if (!agent.loadSkillByName(skillName)) {
      console.log(`Unknown command or skill: /${skillName}`);
      return;
    }

    console.log(`Loaded skill: ${skillName}`);
    if (promptText.length > 0) await this.respondToUser(promptText);
  }

  private async respondToUser(text: string): Promise<void> {
    const agent = this.getAgentOrShowWelcome();
    if (!agent) return;

    const stopSpinner = startSpinner();
    try {
      console.log(await agent.handleUserMessage(text));
    } finally {
      stopSpinner();
    }
  }

  private getAgentOrShowWelcome(): Agent | undefined {
    if (!this.agent) console.log(WELCOME_TEXT);
    return this.agent;
  }
}
