import type {
  JsonValue,
  WorkflowHumanAdapter,
  WorkflowHumanPrompt,
} from "#src/workflows/types.js";

export class SerializedWorkflowHumanAdapter implements WorkflowHumanAdapter {
  private tail: Promise<void> = Promise.resolve();

  constructor(private readonly adapter: WorkflowHumanAdapter) {}

  request(prompt: WorkflowHumanPrompt): Promise<JsonValue> {
    const response = this.tail.then(() => this.adapter.request(prompt));
    this.tail = response.then(
      () => undefined,
      () => undefined,
    );
    return response;
  }
}
