import assert from "node:assert/strict";
import test from "node:test";
import {
  CronExpression,
  validateTimezone,
} from "#src/scheduling/CronExpression.js";

test("parses five-field cron expressions and advances in an IANA timezone", () => {
  const cron = CronExpression.parse("*/15 9-10 * * 1-5");
  const next = cron.next(
    new Date("2026-07-27T14:01:00.000Z"),
    "America/Denver",
  );
  assert.equal(next.toISOString(), "2026-07-27T15:00:00.000Z");
});

test("skips nonexistent local times during daylight-saving transitions", () => {
  const cron = CronExpression.parse("30 2 * * *");
  const next = cron.next(
    new Date("2026-03-08T08:00:00.000Z"),
    "America/Denver",
  );
  assert.equal(next.toISOString(), "2026-03-09T08:30:00.000Z");
});

test("retains both repeated local times when daylight saving ends", () => {
  const cron = CronExpression.parse("30 1 * * *");
  const first = cron.next(
    new Date("2026-11-01T07:29:00.000Z"),
    "America/Denver",
  );
  const second = cron.next(first, "America/Denver");
  assert.equal(first.toISOString(), "2026-11-01T07:30:00.000Z");
  assert.equal(second.toISOString(), "2026-11-01T08:30:00.000Z");
});

test("rejects malformed cron expressions and unknown timezones", () => {
  assert.throws(() => CronExpression.parse("* * * *"), /five fields/);
  assert.throws(() => CronExpression.parse("60 * * * *"), /between 0 and 59/);
  assert.throws(() => validateTimezone("Mars/Olympus_Mons"), /Unknown IANA/);
});
