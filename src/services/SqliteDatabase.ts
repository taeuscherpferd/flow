import type { DatabaseSync } from "node:sqlite";

const SQLITE_BUSY_TIMEOUT_MS = 5_000;

export function configureSqliteDatabase(database: DatabaseSync): void {
  database.exec(`
    PRAGMA busy_timeout = ${SQLITE_BUSY_TIMEOUT_MS};
    PRAGMA journal_mode = WAL;
    PRAGMA foreign_keys = ON;
  `);
}
