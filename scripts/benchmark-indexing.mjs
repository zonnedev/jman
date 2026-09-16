import fs from "node:fs";
import { spawn } from "node:child_process";

const [binary, workspace, buildSystem] = process.argv.slice(2);
const started = process.hrtime.bigint();
const child = spawn(binary, [], {
  env: {
    ...process.env,
    JAVA_LSP_TELEMETRY: "1",
  },
  stdio: ["pipe", "ignore", "pipe"],
});
let stderrBuffer = "";
let completed = false;
child.stderr.on("data", (chunk) => {
  process.stderr.write(chunk);
  stderrBuffer += chunk.toString("utf8");
  const lines = stderrBuffer.split("\n");
  stderrBuffer = lines.pop() ?? "";
  if (
    !completed &&
    lines.some(
      (line) =>
        line.includes('"jman.javaMetric":"workspace-parse"'),
    )
  ) {
    completed = true;
    child.stdin.end();
  }
});
let maxRssKilobytes = 0;
const sample = setInterval(() => {
  try {
    const status = fs.readFileSync(`/proc/${child.pid}/status`, "utf8");
    const rss = /^VmRSS:\s+(\d+)/m.exec(status);
    if (rss) maxRssKilobytes = Math.max(maxRssKilobytes, Number(rss[1]));
  } catch {
    // The process may have exited between the timer and the read.
  }
}, 10);
const message = Buffer.from(
  JSON.stringify({
    jsonrpc: "2.0",
    id: 1,
    method: "initialize",
    params: {
      rootUri: `file://${workspace}`,
      initializationOptions: buildSystem === "auto" ? {} : { buildSystem },
    },
  }),
);
child.stdin.write(
  Buffer.concat([
    Buffer.from(`Content-Length: ${message.length}\r\n\r\n`),
    message,
  ]),
);
const timeout = setTimeout(() => {
  if (!completed) {
    process.stderr.write("indexing benchmark timed out\n");
    child.kill("SIGKILL");
  }
}, 120_000);
child.on("exit", (code) => {
  clearInterval(sample);
  clearTimeout(timeout);
  const elapsedMilliseconds =
    Number(process.hrtime.bigint() - started) / 1_000_000;
  process.stderr.write(
    `${JSON.stringify({
      jman.javaMetric: "process",
      fields: { elapsedMilliseconds, maxRssKilobytes, exitCode: code },
    })}\n`,
  );
  process.exitCode = code ?? 1;
});
