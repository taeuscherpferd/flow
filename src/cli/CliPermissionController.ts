import type { ScheduleCliController } from "#src/cli/ScheduleCliController.js";
import type { ToolEffect } from "#src/tools/types.js";
import { EOF, ghostPrompt } from "#src/ui/lineEditor.js";
import type { JsonObject } from "#src/workflows/types.js";

export interface CliPermissionUi {
  pauseSpinner(): boolean;
  resumeSpinner(): void;
  getScheduleController(): ScheduleCliController | undefined;
}

export type CliPermissionPrompt = (
  prompt: string,
) => Promise<string | typeof EOF>;

export class CliPermissionController {
  private tail: Promise<void> = Promise.resolve();

  constructor(
    private readonly ui: CliPermissionUi,
    private readonly prompt: CliPermissionPrompt = (prompt) =>
      ghostPrompt({
        prompt,
        getCommands: () => [],
      }),
  ) {}

  async request(
    toolName: string,
    args: JsonObject,
    effect?: ToolEffect,
  ): Promise<boolean> {
    let details =
      `${effect ?? "external"} tool: ${toolName}\nArguments:\n` +
      JSON.stringify(args, null, 2);
    if (toolName === "create_schedule") {
      try {
        details =
          this.ui.getScheduleController()?.previewCreation(args) ?? details;
      } catch (error) {
        details += `\nValidation: ${
          error instanceof Error ? error.message : String(error)
        }`;
      }
    }
    return this.confirm(`Allow ${toolName}?`, details);
  }

  confirm(prompt: string, details: string): Promise<boolean> {
    const response = this.tail.then(() =>
      this.requestConfirmation(prompt, details),
    );
    this.tail = response.then(
      () => undefined,
      () => undefined,
    );
    return response;
  }

  private async requestConfirmation(
    prompt: string,
    details: string,
  ): Promise<boolean> {
    const hadSpinner = this.ui.pauseSpinner();
    try {
      console.log(details);
      const answer = await this.prompt(`${prompt} [y/N] `);
      return answer !== EOF && ["y", "yes"].includes(answer.trim().toLowerCase());
    } finally {
      if (hadSpinner) this.ui.resumeSpinner();
    }
  }
}
