// 网络分区：host/port/cdp。输入框保留本地字符串草稿（空串可显示），
// 写 store 时解析为数字（空→0，落盘时 currentConfigDraft 再回落默认值）——复刻旧
// 「input 存原文、readConfigFromUI 时 parseInt || 默认值」的读写语义。

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useAppStore } from "@/shared/store";
import { INPUT_MONO } from "@/shared/lib/ui";

export function NetworkSection() {
  const { t } = useTranslation();
  const config = useAppStore((s) => s.config);
  const setConfigField = useAppStore((s) => s.setConfigField);

  const [host, setHost] = useState("127.0.0.1");
  const [port, setPort] = useState("47823");
  const [cdp, setCdp] = useState("9231");
  const loadedRef = useRef(false);

  // 配置加载完成后一次性灌入本地草稿（之后输入框即事实来源，与旧实现一致）
  useEffect(() => {
    if (config && !loadedRef.current) {
      loadedRef.current = true;
      setHost(config.taskboard_host);
      setPort(String(config.taskboard_port));
      setCdp(String(config.cdp_port));
    }
  }, [config]);

  return (
    <section className="settings-section" id="section-network">
      <h2 className="mb-4 text-base font-semibold">{t("Network")}</h2>

      <div className="flex items-start gap-4 border-b border-border py-4">
        <label className="w-36 shrink-0 pt-2 text-sm font-medium" htmlFor="cfg-host">{t("Taskboard Host")}</label>
        <input type="text" className={`${INPUT_MONO} max-w-xs`} id="cfg-host" value={host}
          onChange={(e) => { setHost(e.target.value); setConfigField({ taskboard_host: e.target.value }); }} />
      </div>

      <div className="flex items-start gap-4 border-b border-border py-4">
        <label className="w-36 shrink-0 pt-2 text-sm font-medium" htmlFor="cfg-port">{t("Taskboard Port")}</label>
        <input type="number" className={`${INPUT_MONO} max-w-[200px]`} id="cfg-port" value={port}
          onChange={(e) => { setPort(e.target.value); setConfigField({ taskboard_port: parseInt(e.target.value, 10) || 0 }); }} />
      </div>

      <div className="flex items-start gap-4 py-4">
        <label className="w-36 shrink-0 pt-2 text-sm font-medium" htmlFor="cfg-cdp">{t("CDP Debug Port")}</label>
        <input type="number" className={`${INPUT_MONO} max-w-[200px]`} id="cfg-cdp" value={cdp}
          onChange={(e) => { setCdp(e.target.value); setConfigField({ cdp_port: parseInt(e.target.value, 10) || 0 }); }} />
      </div>
    </section>
  );
}
