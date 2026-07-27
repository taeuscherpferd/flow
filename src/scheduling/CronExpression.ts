const RANGES = [
  { minimum: 0, maximum: 59 },
  { minimum: 0, maximum: 23 },
  { minimum: 1, maximum: 31 },
  { minimum: 1, maximum: 12 },
  { minimum: 0, maximum: 7 },
] as const;

interface ParsedField {
  values: Set<number>;
  wildcard: boolean;
}

function parseNumber(text: string, minimum: number, maximum: number): number {
  const value = Number(text);
  if (!Number.isInteger(value) || value < minimum || value > maximum) {
    throw new Error(`Cron value "${text}" must be between ${minimum} and ${maximum}.`);
  }
  return value;
}

function parseField(
  text: string,
  minimum: number,
  maximum: number,
  dayOfWeek: boolean,
): ParsedField {
  const values = new Set<number>();
  const wildcard = text === "*" || text.startsWith("*/");
  for (const segment of text.split(",")) {
    const [base, stepText] = segment.split("/");
    if (base === undefined || segment.split("/").length > 2) {
      throw new Error(`Invalid cron field "${text}".`);
    }
    const step =
      stepText === undefined
        ? 1
        : parseNumber(stepText, 1, maximum - minimum + 1);
    let start: number;
    let end: number;
    if (base === "*") {
      start = minimum;
      end = maximum;
    } else if (base.includes("-")) {
      const [startText, endText] = base.split("-");
      if (startText === undefined || endText === undefined) {
        throw new Error(`Invalid cron range "${base}".`);
      }
      start = parseNumber(startText, minimum, maximum);
      end = parseNumber(endText, minimum, maximum);
      if (start > end) throw new Error(`Cron range "${base}" is reversed.`);
    } else {
      start = parseNumber(base, minimum, maximum);
      end = start;
    }
    for (let value = start; value <= end; value += step) {
      values.add(dayOfWeek && value === 7 ? 0 : value);
    }
  }
  return { values, wildcard };
}

interface LocalParts {
  minute: number;
  hour: number;
  day: number;
  month: number;
  weekday: number;
}

const WEEKDAYS: Record<string, number> = {
  Sun: 0,
  Mon: 1,
  Tue: 2,
  Wed: 3,
  Thu: 4,
  Fri: 5,
  Sat: 6,
};

function formatter(timezone: string): Intl.DateTimeFormat {
  try {
    return new Intl.DateTimeFormat("en-US", {
      timeZone: timezone,
      minute: "2-digit",
      hour: "2-digit",
      hourCycle: "h23",
      day: "2-digit",
      month: "2-digit",
      weekday: "short",
    });
  } catch {
    throw new Error(`Unknown IANA timezone "${timezone}".`);
  }
}

function localParts(
  date: Date,
  dateFormatter: Intl.DateTimeFormat,
): LocalParts {
  const parts = Object.fromEntries(
    dateFormatter
      .formatToParts(date)
      .filter((part) => part.type !== "literal")
      .map((part) => [part.type, part.value]),
  );
  return {
    minute: Number(parts["minute"]),
    hour: Number(parts["hour"]),
    day: Number(parts["day"]),
    month: Number(parts["month"]),
    weekday: WEEKDAYS[parts["weekday"] ?? ""] ?? -1,
  };
}

export class CronExpression {
  private constructor(
    readonly source: string,
    private readonly fields: ParsedField[],
  ) {}

  static parse(source: string): CronExpression {
    const parts = source.trim().split(/\s+/);
    if (parts.length !== 5) {
      throw new Error("Cron expression must contain exactly five fields.");
    }
    return new CronExpression(
      parts.join(" "),
      parts.map((part, index) => {
        const range = RANGES[index]!;
        return parseField(
          part!,
          range.minimum,
          range.maximum,
          index === 4,
        );
      }),
    );
  }

  next(after: Date, timezone: string): Date {
    const dateFormatter = formatter(timezone);
    const candidate = new Date(after.getTime());
    candidate.setUTCSeconds(0, 0);
    candidate.setUTCMinutes(candidate.getUTCMinutes() + 1);
    const limit = candidate.getTime() + 366 * 24 * 60 * 60 * 1000 * 8;
    while (candidate.getTime() <= limit) {
      if (this.matches(localParts(candidate, dateFormatter))) {
        return new Date(candidate);
      }
      candidate.setUTCMinutes(candidate.getUTCMinutes() + 1);
    }
    throw new Error("Cron expression has no occurrence in the next eight years.");
  }

  private matches(parts: LocalParts): boolean {
    const [minute, hour, day, month, weekday] = this.fields;
    const dayMatches = day!.values.has(parts.day);
    const weekdayMatches = weekday!.values.has(parts.weekday);
    const calendarDayMatches =
      !day!.wildcard && !weekday!.wildcard
        ? dayMatches || weekdayMatches
        : dayMatches && weekdayMatches;
    return (
      minute!.values.has(parts.minute) &&
      hour!.values.has(parts.hour) &&
      month!.values.has(parts.month) &&
      calendarDayMatches
    );
  }
}

export function validateTimezone(timezone: string): void {
  formatter(timezone);
}
