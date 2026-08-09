import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";

function parseArgs(argv) {
  const options = {
    manifestPath: "",
    filter: "",
    test: "",
    timeout: 60,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index];
    const value = argv[index + 1];
    if (!["--manifest-path", "--filter", "--test", "--timeout"].includes(option)) {
      throw new Error(`unknown option: ${option}`);
    }
    if (value === undefined || value.startsWith("--")) {
      throw new Error(`missing value for ${option}`);
    }
    index += 1;
    if (option === "--manifest-path") options.manifestPath = value;
    if (option === "--filter") options.filter = value;
    if (option === "--test") options.test = value;
    if (option === "--timeout") {
      const timeout = Number(value);
      if (!Number.isSafeInteger(timeout) || timeout <= 0) {
        throw new Error("--timeout must be a positive integer");
      }
      options.timeout = timeout;
    }
  }

  if (!options.manifestPath) throw new Error("--manifest-path is required");
  if (options.filter && options.test) {
    throw new Error("--filter and --test are mutually exclusive");
  }
  return options;
}

function testNames(listOutput) {
  return listOutput
    .split(/\r?\n/u)
    .filter((line) => line.endsWith(": test"))
    .map((line) => line.slice(0, -": test".length));
}

function cargoArgs(options, list) {
  const args = ["test", "--manifest-path", options.manifestPath, "--locked"];
  if (options.test) args.push("--test", options.test);
  if (list) {
    args.push("--", "--list");
    if (options.filter) args.push(options.filter);
  } else if (options.filter) {
    args.push("--", options.filter);
  }
  return args;
}

function disposableEnvironment(home) {
  const realHome = homedir();
  return {
    ...process.env,
    HOME: home,
    USERPROFILE: home,
    CARGO_HOME: process.env.CARGO_HOME ?? join(realHome, ".cargo"),
    RUSTUP_HOME: process.env.RUSTUP_HOME ?? join(realHome, ".rustup"),
  };
}

function killProcessTree(child) {
  if (!child.pid) return;
  if (process.platform === "win32") {
    spawnSync("taskkill.exe", ["/PID", String(child.pid), "/T", "/F"], {
      stdio: "ignore",
      windowsHide: true,
    });
    return;
  }
  try {
    process.kill(-child.pid, "SIGKILL");
  } catch {
    child.kill("SIGKILL");
  }
}

function runWithTimeout(command, args, env, timeoutSeconds) {
  return new Promise((resolve) => {
    const child = spawn(command, args, {
      detached: process.platform !== "win32",
      env,
      stdio: "inherit",
      windowsHide: true,
    });
    let timedOut = false;
    let spawnError = null;
    const timer = setTimeout(() => {
      timedOut = true;
      killProcessTree(child);
    }, timeoutSeconds * 1000);

    child.once("error", (error) => {
      spawnError = error;
    });
    child.once("close", (code) => {
      clearTimeout(timer);
      if (timedOut) {
        console.error(`Rust tests timed out after ${timeoutSeconds} second${timeoutSeconds === 1 ? "" : "s"}`);
        resolve(124);
      } else if (spawnError) {
        console.error(`failed to start Cargo: ${spawnError.message}`);
        resolve(1);
      } else {
        resolve(code ?? 1);
      }
    });
  });
}

async function main() {
  let options;
  try {
    options = parseArgs(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    return 1;
  }

  const disposableHome = mkdtempSync(join(tmpdir(), "dashi-rust-home-"));
  const env = disposableEnvironment(disposableHome);
  const cargo = process.env.CARGO ?? (process.platform === "win32" ? "cargo.exe" : "cargo");
  try {
    const list = spawnSync(cargo, cargoArgs(options, true), {
      encoding: "utf8",
      env,
      maxBuffer: 16 * 1024 * 1024,
      stdio: ["ignore", "pipe", "inherit"],
      windowsHide: true,
    });
    if (list.error) {
      console.error(`failed to list Rust tests: ${list.error.message}`);
      return 1;
    }
    if (list.status !== 0) return list.status ?? 1;

    const names = testNames(list.stdout);
    const matches = options.filter
      ? names.filter((name) => name.startsWith(options.filter))
      : names;
    if (matches.length === 0) {
      console.error("Rust test selection matched zero tests");
      return 2;
    }

    return await runWithTimeout(
      cargo,
      cargoArgs(options, false),
      env,
      options.timeout,
    );
  } finally {
    rmSync(disposableHome, { recursive: true, force: true });
  }
}

process.exitCode = await main();
