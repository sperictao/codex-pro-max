import "@testing-library/jest-dom/vitest";
import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";
import { initI18n } from "@/shared/i18n";

// globals: false 时 RTL 的自动 cleanup 不注册，手动挂上（否则跨测试残留 DOM）
afterEach(() => cleanup());

// 测试统一跑英文界面（key 即原文，断言直接对英文串）
await initI18n("en");
