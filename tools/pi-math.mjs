import { createInterface } from "node:readline";
import { renderLatex } from "../vendor/pi-tui/latex.js";

const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
for await (const line of input) {
  try {
    const request = JSON.parse(line);
    const result = renderLatex(request.source, { display: request.display === true });
    process.stdout.write(JSON.stringify({ result: result ?? null }) + "\n");
  } catch (error) {
    process.stdout.write(JSON.stringify({ result: null, error: String(error) }) + "\n");
  }
}
