import type { AgentManager } from "#src/agents/AgentManager.js";
import type { Tool } from "#src/tools/types.js";

export function createListAgentsTool(manager: AgentManager): Tool {
  return {
    name: "list_agents",
    effect: "read",
    description:
      "List configured specialist agents and their descriptions. This does not switch the active conversation.",
    parameters: { type: "object", properties: {} },
    async execute() {
      const agents = manager.listAgents();
      return {
        ok: true,
        content:
          agents.length === 0
            ? "No specialist agents are configured."
            : agents
                .map(
                  (agent) =>
                    `${agent.name}${agent.active ? " (active)" : ""}: ${agent.description}`,
                )
                .join("\n"),
      };
    },
  };
}

export function createDelegateAgentTool(manager: AgentManager): Tool {
  return {
    name: "delegate_agent",
    effect: "read",
    description:
      "Delegate an explicit task to a configured specialist in a fresh isolated session. Returns only the specialist's final result.",
    parameters: {
      type: "object",
      properties: {
        agent: {
          type: "string",
          description: "Exact configured specialist agent name.",
        },
        task: {
          type: "string",
          description: "A complete, explicit task for the specialist.",
        },
      },
      required: ["agent", "task"],
    },
    async execute(args, _context, signal) {
      const agentName = args["agent"];
      const task = args["task"];
      if (
        typeof agentName !== "string" ||
        typeof task !== "string" ||
        task.trim().length === 0
      ) {
        return {
          ok: false,
          content: "Error: 'agent' and a non-empty 'task' are required.",
        };
      }
      try {
        return {
          ok: true,
          content: await manager.delegate(agentName, task, signal),
        };
      } catch (error) {
        return {
          ok: false,
          content: error instanceof Error ? error.message : String(error),
        };
      }
    },
  };
}
