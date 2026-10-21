import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";

export function allowed(head, base) {
  return !(head === "native" || head.startsWith("build/vz-native-")) || base === "native";
}

if (process.argv[2] === "--test") {
  for (const [head, base, expected] of [
    ["native", "main", false],
    ["build/vz-native-a-scaffold", "main", false],
    ["build/vz-native-b", "release", false],
    ["native", "native", true],
    ["build/vz-native-a-scaffold", "native", true],
    ["build/athena-fix", "main", true],
    ["feature/docker", "release", true],
    ["build/vz-nativeish", "main", true],
  ]) {
    assert.equal(allowed(head, base), expected, `${head} -> ${base}`);
    console.log(`PASS ${head} -> ${base}: ${expected ? "allowed" : "blocked"}`);
  }
} else if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const { HEAD_REF: head, BASE_REF: base } = process.env;
  if (!head || !base) throw new Error("HEAD_REF and BASE_REF are required");
  if (!allowed(head, base)) {
    console.error(`Native branch ${head} must target native (received ${base})`);
    process.exitCode = 1;
  } else console.log(`Allowed: ${head} -> ${base}`);
}
