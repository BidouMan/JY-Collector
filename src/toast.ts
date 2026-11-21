import { reactive } from "vue";

export interface ToastItem {
  id: number;
  text: string;
  kind: "ok" | "error";
}

export const toasts = reactive<ToastItem[]>([]);

let seq = 0;

export function notify(text: string, kind: "ok" | "error" = "ok") {
  const id = ++seq;
  toasts.push({ id, text, kind });
  window.setTimeout(() => {
    const index = toasts.findIndex((item) => item.id === id);
    if (index >= 0) toasts.splice(index, 1);
  }, 2600);
}
