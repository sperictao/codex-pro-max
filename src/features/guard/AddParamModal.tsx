// 添加自定义参数弹窗（旧 guard-add-modal）。
// 联动规则（旧 onGuardAddModeChange/onGuardAddValueTypeChange）：
// - toml_key/toml_absent 显示 TOML Path 与值类型行；file_overwrite/markdown_block 隐藏且 effectiveType 固定 text
// - 值类型 none 隐藏默认值行；值类型 text 时默认值用 textarea（否则 input），内容保留

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useAppStore } from "@/shared/store";
import { Modal } from "@/shared/components/Modal";
import { BTN, BTN_PRIMARY, INPUT, INPUT_MONO, SELECT } from "@/shared/lib/ui";
import * as ops from "./ops";

export function AddParamModal({
  open,
  preferredFileId,
  onClose,
}: {
  open: boolean;
  preferredFileId: string | null;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const guardFiles = useAppStore((s) => s.guardFiles);

  const [id, setId] = useState("");
  const [label, setLabel] = useState("");
  const [fileId, setFileId] = useState("");
  const [mode, setMode] = useState("toml_key");
  const [path, setPath] = useState("");
  const [valueType, setValueType] = useState("bool");
  const [desc, setDesc] = useState("");
  const [defaultRaw, setDefaultRaw] = useState("");

  // 打开时落定目标文件：优先组级预选；其次保留上次有效选择；否则首个文件（旧 renderGuardFileSelect 语义）
  useEffect(() => {
    if (!open) return;
    setFileId((cur) => {
      if (preferredFileId && guardFiles.some((f) => f.id === preferredFileId)) return preferredFileId;
      if (cur && guardFiles.some((f) => f.id === cur)) return cur;
      return guardFiles[0]?.id ?? "";
    });
  }, [open, preferredFileId, guardFiles]);

  const isToml = mode === "toml_key" || mode === "toml_absent";
  const showDefaultRow = !isToml || valueType !== "none";

  const submit = async () => {
    const ok = await ops.addCustom({ id: id.trim(), label: label.trim(), fileId, mode, path: path.trim(), valueType, desc: desc.trim(), defaultRaw });
    if (ok) {
      // 清空表单并收起（旧行为）
      setId("");
      setLabel("");
      setPath("");
      setDesc("");
      setDefaultRaw("");
      onClose();
    }
  };

  return (
    <Modal open={open} onOverlayClick={onClose} cardStyle={{ width: 560 }}>
      <h3 className="text-sm font-semibold">{t("Add Custom Parameter")}</h3>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className="mb-1 block text-xs font-medium">ID</label>
            <div className="flex items-center">
              <span className="rounded-l-md border border-r-0 border-border bg-muted px-2 py-2 font-mono text-xs opacity-70">custom.</span>
              <input type="text" className={`${INPUT_MONO} rounded-l-none`} placeholder="my_param"
                value={id} onChange={(e) => setId(e.target.value)} />
            </div>
          </div>
          <div>
            <label className="mb-1 block text-xs font-medium">{t("Type")}</label>
            <select className={SELECT} value={mode} onChange={(e) => setMode(e.target.value)}>
              <option value="toml_key">{t("toml_key (TOML key value)")}</option>
              <option value="toml_absent">{t("toml_absent (ensure absent)")}</option>
              <option value="file_overwrite">{t("file_overwrite (overwrite whole file)")}</option>
              <option value="markdown_block">{t("markdown_block (marked block)")}</option>
            </select>
          </div>
        </div>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className="mb-1 block text-xs font-medium">{t("Name")}</label>
            <input type="text" className={INPUT} placeholder={t("Display name")}
              value={label} onChange={(e) => setLabel(e.target.value)} />
          </div>
          <div>
            <label className="mb-1 block text-xs font-medium">{t("Target File")}</label>
            <select className={SELECT} value={fileId} onChange={(e) => setFileId(e.target.value)}>
              {guardFiles.map((f) => (
                <option key={f.id} value={f.id}>{f.name} ({f.format})</option>
              ))}
            </select>
          </div>
        </div>
        {isToml && (
          <div>
            <label className="mb-1 block text-xs font-medium">{t("TOML Path")}</label>
            <input type="text" className={INPUT_MONO} placeholder={t("e.g. features.foo.enabled")}
              value={path} onChange={(e) => setPath(e.target.value)} />
          </div>
        )}
        {isToml && (
          <div>
            <label className="mb-1 block text-xs font-medium">{t("Value Type")}</label>
            <select className={SELECT} value={valueType} onChange={(e) => setValueType(e.target.value)}>
              <option value="bool">bool</option>
              <option value="int">int</option>
              <option value="string">string</option>
              <option value="text">{t("text (multi-line text)")}</option>
              <option value="none">{t("none (no value, toml_absent only)")}</option>
            </select>
          </div>
        )}
        {showDefaultRow && (
          <div>
            <label className="mb-1 block text-xs font-medium">{t("Default Value")}</label>
            {valueType === "text" ? (
              <textarea className="guard-form-textarea" value={defaultRaw} onChange={(e) => setDefaultRaw(e.target.value)} />
            ) : (
              <input type="text" className={INPUT} value={defaultRaw} onChange={(e) => setDefaultRaw(e.target.value)} />
            )}
          </div>
        )}
        <div>
          <label className="mb-1 block text-xs font-medium">{t("Description (optional)")}</label>
          <input type="text" className={INPUT} placeholder={t("What this parameter does")}
            value={desc} onChange={(e) => setDesc(e.target.value)} />
        </div>
        <div className="mt-1 flex justify-end gap-2">
          <button className={BTN} onClick={onClose}>{t("Cancel")}</button>
          <button className={BTN_PRIMARY} onClick={() => void submit()}>{t("Add")}</button>
        </div>
    </Modal>
  );
}
