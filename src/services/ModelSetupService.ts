import type { ConfigService, ModelSetup } from "./ConfigService.js";
import { EOF } from "../ui/lineEditor.js";

export type SetupPrompt = (prompt: string) => Promise<string | typeof EOF>;
export type SetupOutput = (message: string) => void;

export interface CompletedModelSetup {
  status: "completed";
  configPath: string;
  provider: string;
  model: string;
}

export interface CancelledModelSetup {
  status: "cancelled";
}

export type ModelSetupResult = CompletedModelSetup | CancelledModelSetup;

const DEFAULT_PROVIDER = "ollama";
const DEFAULT_BASE_URL = "http://localhost:11434";
const DEFAULT_CONTEXT_WINDOW = 8192;

export class ModelSetupService {
  constructor(
    private readonly configService: ConfigService,
    private readonly prompt: SetupPrompt,
    private readonly output: SetupOutput,
  ) {}

  async run(): Promise<ModelSetupResult> {
    this.output("Let's set up your first provider and model.");

    const provider = await this.askProvider();
    if (provider === EOF) return { status: "cancelled" };

    const baseUrl = await this.askBaseUrl();
    if (baseUrl === EOF) return { status: "cancelled" };

    const model = await this.askModel();
    if (model === EOF) return { status: "cancelled" };

    const contextWindow = await this.askContextWindow();
    if (contextWindow === EOF) return { status: "cancelled" };

    const setup: ModelSetup = {
      provider,
      baseUrl,
      model,
      contextWindow,
    };
    const configPath = await this.configService.saveModelSetup(setup);

    return {
      status: "completed",
      configPath,
      provider,
      model,
    };
  }

  private async askProvider(): Promise<string | typeof EOF> {
    for (;;) {
      const answer = await this.prompt(
        `Provider name [${DEFAULT_PROVIDER}]: `,
      );
      if (answer === EOF) return EOF;

      const provider = answer.trim() || DEFAULT_PROVIDER;
      if (!provider.includes("/") && !/\s/.test(provider)) return provider;
      this.output("Provider names cannot contain spaces or slashes.");
    }
  }

  private async askBaseUrl(): Promise<string | typeof EOF> {
    for (;;) {
      const answer = await this.prompt(
        `Ollama-compatible base URL [${DEFAULT_BASE_URL}]: `,
      );
      if (answer === EOF) return EOF;

      const baseUrl = answer.trim() || DEFAULT_BASE_URL;
      if (this.isHttpUrl(baseUrl)) return baseUrl.replace(/\/+$/, "");
      this.output("Enter a valid http:// or https:// URL.");
    }
  }

  private async askModel(): Promise<string | typeof EOF> {
    for (;;) {
      const answer = await this.prompt("Model name: ");
      if (answer === EOF) return EOF;

      const model = answer.trim();
      if (model.length > 0) return model;
      this.output("Model name is required.");
    }
  }

  private async askContextWindow(): Promise<number | typeof EOF> {
    for (;;) {
      const answer = await this.prompt(
        `Context window [${DEFAULT_CONTEXT_WINDOW}]: `,
      );
      if (answer === EOF) return EOF;

      const value = answer.trim();
      if (value.length === 0) return DEFAULT_CONTEXT_WINDOW;

      const contextWindow = Number(value);
      if (Number.isSafeInteger(contextWindow) && contextWindow > 0) {
        return contextWindow;
      }
      this.output("Context window must be a positive whole number.");
    }
  }

  private isHttpUrl(value: string): boolean {
    try {
      const url = new URL(value);
      return url.protocol === "http:" || url.protocol === "https:";
    } catch {
      return false;
    }
  }
}
