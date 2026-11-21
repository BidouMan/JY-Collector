import { reactive } from "vue";
import { api } from "../api";

/** 头像跨页面共享：同一会话内只向本地库取一次，刷新数据时再整体更新。 */
export const avatarStore = reactive({
  map: {} as Record<string, string>,
  loaded: false,
  loading: false,
  attemptedSync: false,
});

function toDataUrl(mime: string, data: string) {
  return `data:${mime || "image/jpeg"};base64,${data}`;
}

export async function loadAvatars(force = false): Promise<void> {
  if (avatarStore.loading) return;
  if (avatarStore.loaded && !force) return;
  avatarStore.loading = true;
  try {
    const list = await api.listAvatars();
    const next: Record<string, string> = {};
    for (const avatar of list) {
      next[avatar.entityId] = toDataUrl(avatar.mime, avatar.data);
    }
    avatarStore.map = next;
    avatarStore.loaded = true;
  } catch {
    // 头像属于辅助信息，读取失败时保持占位头像，不影响采集。
  } finally {
    avatarStore.loading = false;
  }
}

/**
 * 本地库还没有头像时（例如升级后首次进入），单独解一次头像库补齐，
 * 不触发消息重解。整个过程失败都静默，界面继续用姓名首字占位。
 */
export async function ensureAvatars(): Promise<void> {
  await loadAvatars();
  if (!avatarStore.loaded || Object.keys(avatarStore.map).length > 0) return;
  if (avatarStore.attemptedSync) return;
  avatarStore.attemptedSync = true;
  try {
    await api.syncAvatars();
    await loadAvatars(true);
  } catch {
    // 微信未运行、口令缺失等情况下保持占位头像。
  }
}

export function avatarUrl(entityId?: string | null) {
  if (!entityId) return "";
  return avatarStore.map[entityId] ?? "";
}

/** 没有头像时用姓名首字占位。 */
export function avatarInitial(name?: string | null) {
  const text = (name ?? "").trim();
  return text ? text.slice(0, 1).toUpperCase() : "?";
}
