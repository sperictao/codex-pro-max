import { useEffect, useState } from "react";
import { useAppStore, type ToastItem } from "../store";

// Toaster：右下角堆叠，3 秒后 0.3s 淡出移除（行为与旧 toast 一致；JSX 默认转义文本）
export function Toaster() {
  const toasts = useAppStore((s) => s.toasts);
  return (
    <div className="fixed bottom-4 right-4 z-50 flex flex-col gap-2" id="toast-container">
      {toasts.map((item) => (
        <Toast key={item.id} item={item} />
      ))}
    </div>
  );
}

function Toast({ item }: { item: ToastItem }) {
  const [fading, setFading] = useState(false);
  useEffect(() => {
    const timer = setTimeout(() => setFading(true), 3000);
    return () => clearTimeout(timer);
  }, []);
  return (
    <div
      className={`toast ${item.type}`}
      style={fading ? { opacity: 0, transition: "opacity 0.3s" } : undefined}
    >
      {item.message}
    </div>
  );
}
