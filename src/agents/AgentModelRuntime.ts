import { OllamaProvider } from "#src/providers/OllamaProvider.js";
import type { ModelProvider } from "#src/providers/types.js";
import type { ModelsConfig } from "#src/services/ConfigService.js";

export interface ModelReference {
  provider: string;
  model: string;
}

export interface ResolvedModel {
  providerName: string;
  modelName: string;
  contextWindow: number;
}

export function createProviders(
  models: ModelsConfig,
): Map<string, ModelProvider> {
  return new Map(
    Object.entries(models.providers).map(([name, config]) => [
      name,
      new OllamaProvider(config.baseUrl),
    ]),
  );
}

export function listModelReferences(models: ModelsConfig): ModelReference[] {
  return Object.entries(models.providers).flatMap(([provider, config]) =>
    config.models.map((model) => ({ provider, model: model.name })),
  );
}

export function resolveModel(
  models: ModelsConfig,
  requestedSpec: string,
): { ok: true; value: ResolvedModel } | { ok: false; error: string } {
  const spec = models.modelAliases?.[requestedSpec] ?? requestedSpec;
  const slash = spec.indexOf("/");
  let providerName: string;
  let modelName: string;
  if (slash !== -1) {
    providerName = spec.slice(0, slash).trim();
    modelName = spec.slice(slash + 1).trim();
    const config = models.providers[providerName];
    if (!config) {
      return { ok: false, error: `Unknown provider "${providerName}".` };
    }
    if (!config.models.some((model) => model.name === modelName)) {
      return {
        ok: false,
        error: `Provider "${providerName}" has no model "${modelName}".`,
      };
    }
  } else {
    const matches = listModelReferences(models).filter(
      (reference) => reference.model === spec,
    );
    if (matches.length === 0) {
      return { ok: false, error: `Unknown model "${requestedSpec}".` };
    }
    if (matches.length > 1) {
      return {
        ok: false,
        error:
          `Model "${requestedSpec}" exists in multiple providers — qualify it: ` +
          matches
            .map((match) => `${match.provider}/${match.model}`)
            .join(", ") +
          ".",
      };
    }
    providerName = matches[0]!.provider;
    modelName = matches[0]!.model;
  }
  const contextWindow = models.providers[providerName]!.models.find(
    (model) => model.name === modelName,
  )!.contextWindow;
  return {
    ok: true,
    value: { providerName, modelName, contextWindow },
  };
}
