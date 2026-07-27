import { ScheduleWorker } from "#src/scheduling/ScheduleWorker.js";

const globalDir = process.argv[2];
if (!globalDir) throw new Error("Schedule worker requires the global config directory.");

const worker = new ScheduleWorker(globalDir);
let tickRunning = false;

const runTick = async (): Promise<void> => {
  if (tickRunning) return;
  tickRunning = true;
  try {
    await worker.tick();
  } catch (error) {
    console.error(
      `Schedule worker tick failed: ${
        error instanceof Error ? error.message : String(error)
      }`,
    );
  } finally {
    tickRunning = false;
  }
};

const acquiredLease = await worker.tick();
if (!acquiredLease) {
  worker.close();
  process.exit(0);
}
setInterval(runTick, 15_000);
