// 冒烟入口：先装 Tauri mock，再跑正常引导（仅 smoke.html 使用，不进生产 bundle）
import "./install-mock";
import "../main";
