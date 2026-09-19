import "@testing-library/jest-dom/vitest";
import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";
import { initI18n } from "@/shared/i18n";

// globals: false 时 RTL 的自动 cleanup 不注册，手动挂上（否则跨测试残留 DOM）
afterEach(() => cleanup());

// jsdom 不实现 Pointer Events 的捕获相关方法，而 Radix 的 Select/DropdownMenu
// 在 pointerdown 时会调用它们；缺失即抛 "target.hasPointerCapture is not a
// function"，菜单永远打不开。补成 no-op（无捕获语义，但组件流程得以走通）。
if (!Element.prototype.hasPointerCapture) {
  Element.prototype.hasPointerCapture = () => false;
  Element.prototype.setPointerCapture = () => {};
  Element.prototype.releasePointerCapture = () => {};
}

// 菜单滚动定位依赖 scrollIntoView，jsdom 同样没有
if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {};
}

// 测试统一跑英文界面（key 即原文，断言直接对英文串）
await initI18n("en");
