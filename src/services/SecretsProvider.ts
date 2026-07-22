export interface SecretsProvider {
  get(name: string): string | undefined;
  has(name: string): boolean;
}

export class EnvSecretsProvider implements SecretsProvider {
  constructor(envFiles: string[] = []) {
    for (const file of envFiles) {
      try {
        process.loadEnvFile(file);
      } catch {
        console.warn(`Warning: could not load secrets from ${file}.`);
      }
    }
  }

  get(name: string): string | undefined {
    return process.env[name];
  }

  has(name: string): boolean {
    const value = process.env[name];
    return value !== undefined && value.length > 0;
  }
}
