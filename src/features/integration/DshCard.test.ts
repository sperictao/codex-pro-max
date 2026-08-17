import { describe, expect, it } from "vitest";
import type { DshStatus } from "@/shared/types";
import { statusTextKey, timelineFromStatus } from "./DshCard";

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
