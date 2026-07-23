const FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

export function startSpinner(label = ""): () => void {
  if (process.stderr.isTTY !== true) {
    process.stderr.write(`${label}...\n`);
    return () => {};
  }

  let frameIndex = 0;
  process.stderr.write("\x1b[?25l");
  const timer = setInterval(() => {
    const frame = FRAMES[frameIndex % FRAMES.length];
    process.stderr.write(`\r${frame} ${label}...`);
    frameIndex += 1;
  }, 80);

  return () => {
    clearInterval(timer);
    process.stderr.write("\r\x1b[K");
    process.stderr.write("\x1b[?25h");
  };
}
