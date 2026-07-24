import type { JsonObject, JsonValue, WorkflowSchema } from "./types.js";

export interface SchemaValidationResult {
  valid: boolean;
  errors: string[];
}

function isJsonObject(value: JsonValue): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function validateAt(
  schema: WorkflowSchema,
  value: JsonValue,
  path: string,
  errors: string[],
): void {
  if (schema.type === "string") {
    if (typeof value !== "string") {
      errors.push(`${path} must be a string.`);
      return;
    }
    if (schema.enum && !schema.enum.includes(value)) {
      errors.push(`${path} must be one of: ${schema.enum.join(", ")}.`);
    }
    if (schema.minLength !== undefined && value.length < schema.minLength) {
      errors.push(`${path} must contain at least ${schema.minLength} characters.`);
    }
    return;
  }

  if (schema.type === "number") {
    if (typeof value !== "number" || !Number.isFinite(value)) {
      errors.push(`${path} must be a finite number.`);
      return;
    }
    if (schema.minimum !== undefined && value < schema.minimum) {
      errors.push(`${path} must be at least ${schema.minimum}.`);
    }
    if (schema.maximum !== undefined && value > schema.maximum) {
      errors.push(`${path} must be at most ${schema.maximum}.`);
    }
    return;
  }

  if (schema.type === "boolean") {
    if (typeof value !== "boolean") errors.push(`${path} must be a boolean.`);
    return;
  }

  if (schema.type === "array") {
    if (!Array.isArray(value)) {
      errors.push(`${path} must be an array.`);
      return;
    }
    value.forEach((item, index) =>
      validateAt(schema.items, item, `${path}[${index}]`, errors),
    );
    return;
  }

  if (!isJsonObject(value)) {
    errors.push(`${path} must be an object.`);
    return;
  }

  for (const required of schema.required ?? []) {
    if (!(required in value)) {
      errors.push(`${path}.${required} is required.`);
    }
  }

  for (const [key, childValue] of Object.entries(value)) {
    const childSchema = schema.properties[key];
    if (!childSchema) {
      if (schema.additionalProperties === false) {
        errors.push(`${path}.${key} is not allowed.`);
      }
      continue;
    }
    validateAt(childSchema, childValue, `${path}.${key}`, errors);
  }
}

export function validateSchema(
  schema: WorkflowSchema,
  value: JsonValue,
): SchemaValidationResult {
  const errors: string[] = [];
  validateAt(schema, value, "input", errors);
  return { valid: errors.length === 0, errors };
}

export function assertJsonValue(value: JsonValue, label: string): void {
  const visited = new WeakSet<object>();

  function assertAt(current: JsonValue, path: string): void {
    if (
      current === null ||
      typeof current === "string" ||
      typeof current === "boolean"
    ) {
      return;
    }
    if (typeof current === "number") {
      if (!Number.isFinite(current)) {
        throw new Error(`${path} must be a finite number.`);
      }
      return;
    }
    if (typeof current !== "object") {
      throw new Error(`${path} contains a non-JSON value.`);
    }
    if (visited.has(current)) {
      throw new Error(`${path} contains a circular reference.`);
    }
    visited.add(current);

    if (Array.isArray(current)) {
      current.forEach((item, index) => assertAt(item, `${path}[${index}]`));
      visited.delete(current);
      return;
    }

    const prototype = Object.getPrototypeOf(current);
    if (prototype !== Object.prototype && prototype !== null) {
      throw new Error(`${path} must contain only plain JSON objects.`);
    }
    if (Object.getOwnPropertySymbols(current).length > 0) {
      throw new Error(`${path} cannot contain symbol-keyed values.`);
    }
    for (const key of Object.keys(current)) {
      assertAt(Reflect.get(current, key) as JsonValue, `${path}.${key}`);
    }
    visited.delete(current);
  }

  assertAt(value, label);
}
