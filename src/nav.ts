// nav：顶部视图与设置分区的纯 DOM 切换（叶子模块，不 import 任何域）
// 进入某视图后的数据刷新由 shell 的 wireEvents 在调用点触发
// （ADR 0009：跨域通信集中在事件接线）

export function toggleSettings(): void {
  const mainView = document.getElementById("main-view")!;
  const settingsView = document.getElementById("settings-view")!;
  const btn = document.getElementById("btn-settings")!;
  const homeBtn = document.getElementById("btn-home")!;

  const isHidden = settingsView.classList.contains("hidden");

  if (isHidden) {
    mainView.classList.add("hidden");
    document.getElementById("skill-view")!.classList.add("hidden");
    document.getElementById("guard-view")!.classList.add("hidden");
    document.getElementById("integration-view")!.classList.add("hidden");
    document.getElementById("btn-skill")!.classList.remove("active");
    document.getElementById("btn-guard")!.classList.remove("active");
    document.getElementById("btn-integration")!.classList.remove("active");
    settingsView.classList.remove("hidden");
    btn.classList.add("active");
    homeBtn.classList.remove("active");
  } else {
    showHome();
  }
}

export function showHome(): void {
  document.getElementById("main-view")!.classList.remove("hidden");
  document.getElementById("settings-view")!.classList.add("hidden");
  document.getElementById("skill-view")!.classList.add("hidden");
  document.getElementById("guard-view")!.classList.add("hidden");
  document.getElementById("integration-view")!.classList.add("hidden");
  document.getElementById("btn-settings")!.classList.remove("active");
  document.getElementById("btn-skill")!.classList.remove("active");
  document.getElementById("btn-guard")!.classList.remove("active");
  document.getElementById("btn-integration")!.classList.remove("active");
  document.getElementById("btn-home")!.classList.add("active");
}

export function showSkill(): void {
  document.getElementById("main-view")!.classList.add("hidden");
  document.getElementById("settings-view")!.classList.add("hidden");
  document.getElementById("guard-view")!.classList.add("hidden");
  document.getElementById("integration-view")!.classList.add("hidden");
  document.getElementById("skill-view")!.classList.remove("hidden");
  document.getElementById("btn-settings")!.classList.remove("active");
  document.getElementById("btn-home")!.classList.remove("active");
  document.getElementById("btn-guard")!.classList.remove("active");
  document.getElementById("btn-integration")!.classList.remove("active");
  document.getElementById("btn-skill")!.classList.add("active");
}

export function showGuard(): void {
  document.getElementById("main-view")!.classList.add("hidden");
  document.getElementById("settings-view")!.classList.add("hidden");
  document.getElementById("skill-view")!.classList.add("hidden");
  document.getElementById("integration-view")!.classList.add("hidden");
  document.getElementById("guard-view")!.classList.remove("hidden");
  document.getElementById("btn-settings")!.classList.remove("active");
  document.getElementById("btn-home")!.classList.remove("active");
  document.getElementById("btn-skill")!.classList.remove("active");
  document.getElementById("btn-integration")!.classList.remove("active");
  document.getElementById("btn-guard")!.classList.add("active");
}

// 顶部导航直达集成页：已在集成页则回主页，否则切过去
export function showIntegration(): void {
  const integrationView = document.getElementById("integration-view")!;
  const alreadyThere = !integrationView.classList.contains("hidden");

  if (alreadyThere) {
    showHome();
    return;
  }

  document.getElementById("main-view")!.classList.add("hidden");
  document.getElementById("settings-view")!.classList.add("hidden");
  document.getElementById("skill-view")!.classList.add("hidden");
  document.getElementById("guard-view")!.classList.add("hidden");
  integrationView.classList.remove("hidden");
  document.getElementById("btn-settings")!.classList.remove("active");
  document.getElementById("btn-home")!.classList.remove("active");
  document.getElementById("btn-skill")!.classList.remove("active");
  document.getElementById("btn-guard")!.classList.remove("active");
  document.getElementById("btn-integration")!.classList.add("active");
}

export function switchSection(section: string): void {
  document.querySelectorAll(".settings-section").forEach((el) => {
    el.classList.add("hidden");
  });
  document.getElementById(`section-${section}`)!.classList.remove("hidden");

  document.querySelectorAll(".nav-item").forEach((el) => {
    el.classList.remove("active");
  });
  document.getElementById(`nav-${section}`)!.classList.add("active");

  const footer = document.getElementById("settings-footer")!;
  if (section === "about" || section === "appearance" || section === "guard") {
    footer.classList.add("hidden");
  } else {
    footer.classList.remove("hidden");
  }
}
