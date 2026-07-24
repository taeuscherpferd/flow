import type {
  ChatMessage,
  ModelProvider,
  ThinkingMode,
} from "#src/providers/types.js";
import type { AgentComsService } from "#src/services/AgentComsService.js";
import type {
  WorkflowAgentRunOptions,
  WorkflowThinking,
} from "#src/workflows/types.js";

export interface AgentSessionModel {
  provider: string;
  model: string;
}

function toProviderThinking(
  thinking: WorkflowThinking | undefined,
): ThinkingMode | undefined {
  return thinking;
}

export class AgentSession {
  constructor(
    readonly id: string,
    private providerName: string,
    private readonly agentComs: AgentComsService,
  ) {}

  getModel(): AgentSessionModel {
    return {
      provider: this.providerName,
      model: this.agentComs.getModel(),
    };
  }

  async run(
    prompt: string,
    options: WorkflowAgentRunOptions = {},
    signal?: AbortSignal,
  ): Promise<string> {
    const thinking = toProviderThinking(options.thinking);
    return this.agentComs.handleUserMessage(prompt, {
      tools: options.tools ?? "default",
      ...(thinking === undefined ? {} : { thinking }),
    }, signal);
  }

  snapshotHistory(): ChatMessage[] {
    return this.agentComs.snapshotHistory();
  }

  restoreHistory(history: ChatMessage[]): void {
    this.agentComs.restoreHistory(history);
  }

  retarget(
    providerName: string,
    provider: ModelProvider,
    model: string,
    contextWindow: number,
  ): void {
    this.providerName = providerName;
    this.agentComs.setModel(provider, model, contextWindow);
  }
}
