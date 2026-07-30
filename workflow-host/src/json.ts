import type { JsonObject, JsonValue } from "./types.js";

export function isJsonObject(value: JsonValue): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function assertJsonValue(
  value: JsonValue,
  label: string,
): void {
  const visited = new WeakSet<object>();

  const assertAt = (current: JsonValue, path: string): void => {
    if (
      current === null ||
      typeof current === "string" ||
      typeof current === "boolean"
    ) {
      return;
    }
    if (typeof current === "number") {
      if (!Number.isFinite(current)) {
        throw new TypeError(`${path} must be a finite number.`);
      }
      return;
    }
    if (typeof current !== "object") {
      throw new TypeError(`${path} contains a non-JSON value.`);
    }
    if (visited.has(current)) {
      throw new TypeError(`${path} contains a circular reference.`);
    }
    visited.add(current);
    if (Array.isArray(current)) {
      current.forEach((item, index) => assertAt(item, `${path}[${index}]`));
    } else {
      const prototype = Object.getPrototypeOf(current);
      if (prototype !== Object.prototype && prototype !== null) {
        throw new TypeError(`${path} must contain only plain JSON objects.`);
      }
      if (Object.getOwnPropertySymbols(current).length > 0) {
        throw new TypeError(`${path} cannot contain symbol-keyed values.`);
      }
      for (const [key, item] of Object.entries(current)) {
        assertAt(item, `${path}.${key}`);
      }
    }
    visited.delete(current);
  };

  assertAt(value, label);
}
