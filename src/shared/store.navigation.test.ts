import { beforeEach, describe, expect, it } from "vitest";
import { useAppStore } from "./store";

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
});

describe("navigation", () => {
  it("goto is idempotent and only changes settings section when one is supplied", () => {
    useAppStore.getState().goto("settings", "appearance");
    expect(useAppStore.getState().activeView).toBe("settings");
    expect(useAppStore.getState().settingsSection).toBe("appearance");

    useAppStore.getState().goto("settings");
    useAppStore.getState().goto("settings");

    expect(useAppStore.getState().activeView).toBe("settings");
    expect(useAppStore.getState().settingsSection).toBe("appearance");
  });
});
