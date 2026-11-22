import { reactive } from "vue";
import { api } from "../api";

/** 头像跨页面共享：本地缓存优先，缺失时再按需从 CDN 下载。 */
export const avatarStore = reactive({
  /** 微信头像库里直接解出的头像 */
  map: {} as Record<string, string>,
  /** 按需下载后缓存在内存里的头像 */
  fetched: {} as Record<string, string>,
  /** 联系人库里的头像地址（群成员也能覆盖到） */
  urls: {} as Record<string, string>,
  loaded: false,
  loading: false,
  attemptedSync: false,
});

const pending = new Set<string>();
const failed = new Set<string>();
const queue: string[] = [];
const MAX_CONCURRENT = 4;
let active = 0;

function toDataUrl(mime: string, data: string) {
  return `data:${mime || "image/jpeg"};base64,${data}`;
}

export async function loadAvatars(force = false): Promise<void> {
  if (avatarStore.loading) return;
  if (avatarStore.loaded && !force) return;
  avatarStore.loading = true;
  try {
    const [list, urls] = await Promise.all([api.listAvatars(), api.listAvatarUrls()]);
    const next: Record<string, string> = {};
    for (const avatar of list) {
      next[avatar.entityId] = toDataUrl(avatar.mime, avatar.data);
    }
    avatarStore.map = next;
    const nextUrls: Record<string, string> = {};
    for (const item of urls) {
      nextUrls[item.entityId] = item.url;
    }
    avatarStore.urls = nextUrls;
    avatarStore.loaded = true;
    if (force) failed.clear();
  } catch {
    // 头像属于辅助信息，读取失败时保持占位头像，不影响主流程。
  } finally {
    avatarStore.loading = false;
  }
}

/**
 * 本地头像或头像地址为空时（例如升级后首次进入），单独解一次微信头像库
 * 与联系人库补齐；整个过程失败都静默，界面继续用姓名首字占位。
 */
export async function ensureAvatars(): Promise<void> {
  await loadAvatars();
  const hasData = Object.keys(avatarStore.map).length > 0;
  const hasUrls = Object.keys(avatarStore.urls).length > 0;
  if (hasData && hasUrls) return;
  if (avatarStore.attemptedSync) return;
  avatarStore.attemptedSync = true;
  try {
    await api.syncAvatars();
    await loadAvatars(true);
  } catch {
    // 微信未运行、口令缺失等情况下保持占位头像。
  }
}

/**
 * 为当前可见的消息/会话批量补齐头像。只请求缺头像的人，并限制并发，
 * 下载完成后写入响应式 map，界面自动刷新。
 */
export function ensureAvatarsFor(ids: (string | null | undefined)[]): void {
  for (const raw of ids) {
    const id = (raw ?? "").trim();
    if (!id || avatarStore.map[id] || avatarStore.fetched[id]) continue;
    if (!avatarStore.urls[id] || pending.has(id) || failed.has(id)) continue;
    pending.add(id);
    queue.push(id);
  }
  pump();
}

function pump() {
  while (active < MAX_CONCURRENT && queue.length > 0) {
    const id = queue.shift() as string;
    active += 1;
    void api
      .loadAvatar(id)
      .then((payload) => {
        avatarStore.fetched[id] = toDataUrl(payload.mime, payload.data);
      })
      .catch(() => {
        failed.add(id);
      })
      .finally(() => {
        pending.delete(id);
        active -= 1;
        pump();
      });
  }
}

export function avatarUrl(entityId?: string | null) {
  if (!entityId) return "";
  return avatarStore.map[entityId] ?? avatarStore.fetched[entityId] ?? "";
}

/** 没有头像时用姓名首字占位。 */
export function avatarInitial(name?: string | null) {
  const text = (name ?? "").trim();
  return text ? text.slice(0, 1).toUpperCase() : "?";
}
