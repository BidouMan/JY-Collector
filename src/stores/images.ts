import { api } from "../api";

/** 图片按 md5 缓存，切会话或反复渲染时不会重复解密。 */
const cache = new Map<string, string>();
const pending = new Map<string, Promise<string>>();
/** 上限按条数控制（缩略图/表情都在几百 KB 内），超出的最久未用项直接丢弃。 */
const MAX_CACHE_ENTRIES = 300;

function cacheKey(md5: string, full: boolean) {
  return `${full ? "full" : "thumb"}:${md5}`;
}

function cacheGet(key: string): string {
  const url = cache.get(key);
  if (url === undefined) return "";
  // Map 保持插入顺序，命中后挪到末尾即为 LRU。
  cache.delete(key);
  cache.set(key, url);
  return url;
}

function cacheSet(key: string, url: string) {
  cache.delete(key);
  cache.set(key, url);
  while (cache.size > MAX_CACHE_ENTRIES) {
    const oldest = cache.keys().next().value;
    if (oldest === undefined) break;
    cache.delete(oldest);
  }
}

export async function loadImageUrl(md5: string, full = false): Promise<string> {
  const key = cacheKey(md5, full);
  const cached = cacheGet(key);
  if (cached) return cached;
  const running = pending.get(key);
  if (running) return running;

  const task = (async () => {
    const payload = await api.loadMessageImage(md5, full);
    const url = `data:${payload.mime};base64,${payload.data}`;
    cacheSet(key, url);
    return url;
  })();

  pending.set(key, task);
  try {
    return await task;
  } finally {
    pending.delete(key);
  }
}

export function cachedImageUrl(md5: string, full = false) {
  return cacheGet(cacheKey(md5, full));
}

/** 表情按 md5 缓存；同一条消息只下载一次。 */
export async function loadEmoticonUrl(messageId: string, md5?: string | null): Promise<string> {
  const key = `emo:${md5 || messageId}`;
  const cached = cacheGet(key);
  if (cached) return cached;
  const running = pending.get(key);
  if (running) return running;

  const task = (async () => {
    const payload = await api.loadMessageEmoticon(messageId);
    const url = `data:${payload.mime};base64,${payload.data}`;
    cacheSet(key, url);
    return url;
  })();

  pending.set(key, task);
  try {
    return await task;
  } finally {
    pending.delete(key);
  }
}

/** 本地数据整体刷新后，图片内容可能变化，清掉旧缓存。 */
export function clearImageCache() {
  cache.clear();
}
