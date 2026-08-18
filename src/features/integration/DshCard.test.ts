import { describe, expect, it } from "vitest";
import type { DshStatus } from "@/shared/types";
import { localStatusTextKey, localTimelineFromStatus, statusTextKey, timelineFromStatus } from "./DshCard";

const ready: DshStatus = {
  nodeAvailable: true,
  dshInstalled: true,
  dshVersion: "0.1.0-rc.6",
  supportedVersion: "0.1.0-rc.6",
  dshCompatible: true,
  pluginsInstalled: true,
  dshRunning: true,
  tailscaleInstalled: true,
  tailscaleOnline: true,
  hostname: "node",
  localUrl: "http://127.0.0.1:3899",
  url: "https://node.tailnet.ts.net",
  magicDnsEnabled: true,
  serveConfigured: true,
  autostartEnabled: false,
  error: null,
};

describe("dsh auth plugin readiness", () => {
  it("includes plugin installation and no loopback proxy step", () => {
    expect(timelineFromStatus(ready).map((step) => step.id)).toEqual([
      "node",
      "install",
      "plugins",
      "tailscale",
      "magicdns",
      "start",
      "serve",
      "verify",
    ]);
    expect(timelineFromStatus(ready).every((step) => step.state === "done")).toBe(true);
  });

  it("reports missing auth plugins only once dsh web is running", () => {
    // 插件只服务于远程访问链路：dsh 尚未运行时优先报告未运行
    expect(statusTextKey({ ...ready, pluginsInstalled: false, dshRunning: false }))
      .toBe("dsh web not running");
    // 本地模式已运行时，未装插件意味着远程还没准备好
    expect(statusTextKey({ ...ready, pluginsInstalled: false, dshRunning: true }))
      .toBe("dsh auth plugins not installed");
  });

  it("reports an incompatible dsh core before plugin state", () => {
    expect(statusTextKey({ ...ready, dshCompatible: false, pluginsInstalled: false }))
      .toBe("dsh version is not supported by the auth plugins");
  });
});

describe("local one-click timeline", () => {
  it("uses 4 local-only steps and marks all done when dsh web is running", () => {
    const steps = localTimelineFromStatus(ready);
    expect(steps.map((s) => s.id)).toEqual(["node", "install", "start", "ready"]);
    expect(steps.map((s) => s.index)).toEqual([0, 1, 2, 3]);
    expect(steps.every((s) => s.state === "done")).toBe(true);
  });

  it("marks start/ready pending when dsh web is not running", () => {
    const steps = localTimelineFromStatus({ ...ready, dshRunning: false });
    expect(steps[2].state).toBe("pending");
    expect(steps[3].state).toBe("pending");
  });
});

describe("local mode status text", () => {
  it("ignores remote-only prerequisites and reports ready when dsh web runs", () => {
    // 本地模式不要求插件/Tailscale：插件缺失即可就绪
    expect(localStatusTextKey({ ...ready, pluginsInstalled: false, serveConfigured: false }))
      .toBe("Local access ready");
    expect(statusTextKey({ ...ready, pluginsInstalled: false, serveConfigured: false }))
      .toBe("dsh auth plugins not installed");
  });

  it("reports missing prerequisites in local order", () => {
    expect(localStatusTextKey({ ...ready, dshRunning: false })).toBe("dsh web not running");
    expect(localStatusTextKey({ ...ready, nodeAvailable: false })).toBe("Node.js not detected");
    expect(localStatusTextKey({ ...ready, dshInstalled: false, dshCompatible: false }))
      .toBe("DeepSeek Harness not installed");
  });
});
