#!/usr/bin/env python3
"""微信 4.x 本地库 -> 结构化群/消息 JSON 的 sidecar。

调用方式（由 Tauri 后端拉起，密钥经 stdin 传入，不落在命令行/环境变量/日志）：
  echo '{"keyHex":"...","dbRoot":"...","since":{"<群username>":<epoch>}}' | python3 wechat_export.py

输出：stdout 仅输出一个 JSON 对象；进度与错误走 stderr。
约束：
- 对微信库只读：只读取原文件与 -wal，解密结果写到系统临时目录，退出时删除。
- 不写任何解密产物到仓库目录。
- 密钥不打印、不落盘。
"""

from __future__ import annotations

import base64
import hashlib
import html
import json
import os
import re
import shutil
import sqlite3
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import sqlcipher4 as sc
import wal as walmod
import windows_keys
from wechat_msg import ZSTD_MAGIC, _maybe_zstd, decode_content, _xml_field

DEFAULT_DB_ROOT = os.path.expanduser(
    "~/Library/Containers/com.tencent.xinWeChat/Data/Documents/xwechat_files"
)
PAGE = sc.PAGE_SIZE
KDF_ITER = 256000
# 首次全量时每个群最多取多少条（按时间倒序截取），避免一次性灌入过多数据。
MAX_MESSAGES_PER_GROUP = 2000

_BASE_MASK = 0xFFFFFFFF


def log(msg: str) -> None:
    print(msg, file=sys.stderr, flush=True)


def _derive(passphrase: bytes, salt: bytes) -> tuple[bytes, bytes]:
    """口令 -> (enc_key, hmac_key)。微信 4.x 的口令是 32 字节口令，
    仍需走 PBKDF2-HMAC-SHA512(iter=256000)；派生后的 enc_key 才是页加密密钥。
    """
    enc_key = hashlib.pbkdf2_hmac("sha512", passphrase, salt, KDF_ITER, 32)
    hmac_salt = bytes(b ^ sc.HMAC_SALT_MASK for b in salt)
    hmac_key = hashlib.pbkdf2_hmac("sha512", enc_key, hmac_salt, sc.FAST_KDF_ITER, 32)
    return enc_key, hmac_key


def _has_account(root: str) -> bool:
    if not root or not os.path.isdir(root):
        return False
    try:
        for entry in os.listdir(root):
            if os.path.isfile(
                    os.path.join(root, entry, "db_storage", "contact", "contact.db")):
                return True
    except OSError:
        return False
    return False


def _windows_db_root() -> str:
    """Windows 默认 Documents\\xwechat_files；也读取微信配置里的自定义位置。"""
    candidates: list[str] = []
    env = os.environ.get("JY_WECHAT_DB_ROOT", "").strip()
    if env:
        candidates.append(env)
    home = os.path.expanduser("~")
    candidates.append(os.path.join(home, "Documents", "xwechat_files"))
    candidates.append(os.path.join(home, "OneDrive", "Documents", "xwechat_files"))

    appdata = os.environ.get("APPDATA", "")
    config_dir = os.path.join(appdata, "Tencent", "xwechat", "config")
    if appdata and os.path.isdir(config_dir):
        try:
            for name in os.listdir(config_dir):
                if not name.lower().endswith(".ini"):
                    continue
                try:
                    with open(os.path.join(config_dir, name), "r",
                              encoding="utf-8", errors="ignore") as file:
                        text = file.read()
                except OSError:
                    continue
                for match in re.finditer(r"([A-Za-z]:[\\/][^\r\n\"']*)", text):
                    raw = match.group(1).strip().rstrip("\\/")
                    candidates.append(raw)
                    candidates.append(os.path.join(raw, "xwechat_files"))
        except OSError:
            pass

    for candidate in candidates:
        if _has_account(candidate):
            return candidate
    return candidates[0] if candidates else os.path.join(
        home, "Documents", "xwechat_files")


def resolve_db_root(request: dict) -> str:
    """请求里带了有效目录就用它；否则按平台自动探测。"""
    root = (request.get("dbRoot") or "").strip()
    if root and os.path.isdir(root):
        if sys.platform != "win32" or _has_account(root):
            return root
        detected = _windows_db_root()
        return detected if _has_account(detected) else root
    if sys.platform == "win32":
        return _windows_db_root()
    return root or DEFAULT_DB_ROOT


class KeyRing:
    """按数据库提供 (enc_key, hmac_key)。

    macOS：单个口令对每个库走 PBKDF2 派生。
    Windows：WCDB 在内存里缓存的是每个库派生后的原始 enc_key，按库的 salt 匹配，
    不再做 PBKDF2；找不到时用首页 HMAC 校验兜底。
    """

    def __init__(self, key_hex: str = ""):
        key_hex = (key_hex or "").strip().lower().removeprefix("0x")
        self._passphrase = bytes.fromhex(key_hex) if len(key_hex) == 64 else None
        if self._passphrase is None and sys.platform != "win32":
            raise RuntimeError("密钥格式不正确（需要 32 字节口令）")
        self._pairs: dict[str, str] | None = None
        self._cache: dict[str, tuple[bytes, bytes]] = {}

    def _windows_pairs(self) -> dict[str, str]:
        if self._pairs is None:
            pairs: dict[str, str] = {}
            for salt_hex, key_hex in windows_keys.scan_key_pairs():
                pairs.setdefault(salt_hex, key_hex)
            if not pairs:
                raise RuntimeError("未从微信进程读取到数据库密钥")
            self._pairs = pairs
        return self._pairs

    def ensure(self) -> None:
        """Windows 下提前扫描内存；密钥不可用时抛出可读错误。"""
        if self._passphrase is None:
            self._windows_pairs()

    def _match_key(self, db_path: str, salt: bytes) -> tuple[str, bool]:
        """返回 (key_hex, 是否原始密钥)。兼容缓存里存原始密钥或口令两种情况。"""
        pairs = self._windows_pairs()
        salt_hex = salt.hex()
        salt_match = pairs.get(salt_hex)
        ordered: list[str] = []
        if salt_match:
            ordered.append(salt_match)
        ordered.extend(key for key in pairs.values() if key not in ordered)
        page: bytes | None = None
        for key_hex in ordered:
            key = bytes.fromhex(key_hex)
            if page is None:
                with open(db_path, "rb") as file:
                    page = file.read(PAGE)
            if len(page) != PAGE:
                break
            _, raw_hmac = sc.derive_keys(key, salt)
            if sc.verify_page(page, 1, raw_hmac):
                return key_hex, True
        # 少数版本缓存的是口令而不是派生后的密钥；仅对 salt 匹配项尝试，避免拖慢。
        if salt_match:
            key = bytes.fromhex(salt_match)
            _, pass_hmac = _derive(key, salt)
            with open(db_path, "rb") as file:
                page = file.read(PAGE)
            if len(page) == PAGE and sc.verify_page(page, 1, pass_hmac):
                return salt_match, False
        raise RuntimeError("没有找到这个数据库的密钥，请确认微信已登录后重试")

    def keys(self, db_path: str) -> tuple[bytes, bytes]:
        cached = self._cache.get(db_path)
        if cached is not None:
            return cached
        salt = sc.read_salt(db_path)
        if self._passphrase is not None:
            derived = _derive(self._passphrase, salt)
        else:
            key_hex, is_raw = self._match_key(db_path, salt)
            key = bytes.fromhex(key_hex)
            derived = sc.derive_keys(key, salt) if is_raw else _derive(key, salt)
        self._cache[db_path] = derived
        return derived


def _read_pages(path: str) -> dict[int, bytes]:
    pages: dict[int, bytes] = {}
    with open(path, "rb") as f:
        blob = f.read()
    for offset in range(0, len(blob), PAGE):
        page = blob[offset:offset + PAGE]
        if len(page) == PAGE:
            pages[offset // PAGE + 1] = page
    wal_path = path + "-wal"
    if os.path.exists(wal_path):
        _, wal_pages = walmod.read_wal_pages(wal_path, filter_salt=True)
        pages.update(wal_pages)
    return pages


def _decrypt_to(db_path: str, out_path: str, keys: KeyRing) -> int:
    """解密整库到临时文件，返回 HMAC 校验失败的页数。"""
    enc_key, hmac_key = keys.keys(db_path)
    pages = _read_pages(db_path)
    bad = 0
    with open(out_path, "wb") as f:
        for pgno in sorted(pages):
            page = pages[pgno]
            if sc.verify_page(page, pgno, hmac_key):
                f.write(sc.decrypt_page(page, pgno, enc_key))
            else:
                bad += 1
                f.write(bytes(PAGE))
    return bad


def _open_plain(path: str) -> sqlite3.Connection:
    return sqlite3.connect(f"file:{path}?mode=ro&immutable=1", uri=True)


def _tables(con: sqlite3.Connection) -> list[str]:
    return [r[0] for r in con.execute(
        "SELECT name FROM sqlite_master WHERE type='table'")]


def _key_ok(db_path: str, keys: KeyRing) -> bool:
    try:
        _, hmac_key = keys.keys(db_path)
        with open(db_path, "rb") as f:
            page = f.read(PAGE)
        return len(page) == PAGE and sc.verify_page(page, 1, hmac_key)
    except Exception:
        return False


def _pick_account(db_root: str, keys: KeyRing) -> str | None:
    if not os.path.isdir(db_root):
        return None
    for entry in sorted(os.listdir(db_root)):
        contact = os.path.join(db_root, entry, "db_storage", "contact", "contact.db")
        if os.path.isfile(contact) and _key_ok(contact, keys):
            return entry
    return None


def _clean(text: str) -> str:
    return text.replace("\x00", "").strip()


def _strip_sender_prefix(text: str, sender: str) -> str:
    for prefix in (f"{sender}:\n", f"{sender}:"):
        if text.startswith(prefix):
            return text[len(prefix):].lstrip("\n")
    return text


def _human_size(num: int | None) -> str | None:
    if not num or num <= 0:
        return None
    value = float(num)
    for unit in ("B", "KB", "MB", "GB"):
        if value < 1024 or unit == "GB":
            return f"{value:.1f} {unit}" if unit != "B" else f"{int(value)} B"
        value /= 1024
    return None


def _strip_zstd(raw: bytes) -> bytes:
    if raw.startswith(ZSTD_MAGIC):
        inflated = _maybe_zstd(raw)
        if inflated is not None:
            return inflated
    return raw


def _sanitize_label(value: str | None) -> str:
    """标题里偶尔会嵌套一段 XML（例如转发内容），只取内层可读标题。"""
    text = (value or "").strip()
    if not text:
        return ""
    if text.startswith("<"):
        inner = _xml_field(text, "title") or _xml_field(text, "des") or ""
        if inner.strip().startswith("<"):
            return ""
        text = inner
    return _clean(text.splitlines()[0]) if text else ""


def _decode_emoticon(raw) -> dict:
    """表情消息：保留 md5 与原始下载地址，供界面按需取图。

    本地表情缓存是加密的，且没有稳定可派生的密钥；消息 XML 里的 cdnurl
    可以直接取到原图，因此这里记录地址，由 Rust 侧按需下载（不落盘）。
    """
    blob = raw
    if isinstance(blob, str):
        blob = blob.encode("utf-8", "surrogateescape")
    if blob is None:
        return {"type": "text", "content": "[表情]"}
    blob = _strip_zstd(blob)
    text = _as_text(blob)
    emoji = re.search(r"<emoji[^>]*>", text)
    if not emoji:
        return {"type": "text", "content": "[表情]"}
    attrs = dict(re.findall(r'(\w+)="([^"]*)"', emoji.group(0)))
    url = (attrs.get("thumburl") or attrs.get("tpurl")
           or attrs.get("cdnurl") or attrs.get("encrypturl") or "")
    return {
        "type": "emoticon",
        "content": _clean(html.unescape(attrs.get("desc") or "")) or "[表情]",
        "emoticonMd5": (attrs.get("md5") or "").lower() or None,
        "emoticonUrl": html.unescape(url) or None,
        "emoticonKey": (attrs.get("aeskey") or "").lower() or None,
    }


def _decode_message(raw, local_type: int, sender: str) -> dict:
    """把一条 message_content 归一化为界面需要的 (type, content, quote, file...)。"""
    base = local_type & _BASE_MASK
    sub = local_type >> 32

    # 图片/语音等媒体消息不解析正文
    if base == 3:
        return {"type": "image", "content": "", "fileName": "微信图片"}
    if base == 34:
        return {"type": "text", "content": "[语音]"}
    if base == 43:
        return {"type": "text", "content": "[视频]"}
    if base == 47:
        return _decode_emoticon(raw)
    if base == 42:
        return {"type": "text", "content": "[名片]"}
    if base == 48:
        return {"type": "text", "content": "[位置]"}
    if base == 50:
        return {"type": "text", "content": "[通话]"}
    if base in (10000, 10002):
        return {"type": "skip", "content": ""}

    blob = raw
    if isinstance(blob, str):
        blob = blob.encode("utf-8", "surrogateescape")
    if blob is None:
        return {"type": "text", "content": ""}

    blob = _strip_zstd(blob)

    # 群消息正文通常带 "<发送人username>:\n" 前缀；去掉后再判断是纯文本还是 XML。
    text = _as_text(blob)
    stripped = _strip_sender_prefix(text, sender)
    is_xml = stripped.lstrip().startswith("<")

    # 普通文本自己解码：微信正文里常带 U+2005 之类的不可打印空格，
    # 不能交给 wechat_msg 的 printable 判定（会被误判成 protobuf）。
    if base == 1 and not is_xml:
        return {"type": "text", "content": _clean(stripped)}

    if is_xml:
        blob = stripped.encode("utf-8")

    decoded = decode_content(blob, base)
    text = _clean(decoded.get("text") or "")
    refer = decoded.get("refer")

    if refer:
        quoted = _strip_sender_prefix(_clean(refer.get("content") or ""), sender)
        return {
            "type": "quote",
            "content": text,
            "quotedSender": _clean(refer.get("displayname") or "") or None,
            "quotedContent": quoted or None,
        }

    if decoded.get("kind") == "xml":
        # 应用消息：链接 / 文件 / 小程序等都归一成可读文本或文件条目
        app_type = (decoded.get("app_type") or "").strip()
        xml_text = _as_text(blob)
        label = (_sanitize_label(_xml_field(xml_text, "title"))
                 or _sanitize_label(_xml_field(xml_text, "des")))
        if app_type == "6" or sub == 8:
            size = _human_size(_to_int(_xml_field(xml_text, "totallen")))
            return {"type": "file", "content": "", "fileName": label or "文件",
                    "fileSize": size}
        if app_type == "57":
            return {"type": "text", "content": label or "[引用消息]"}
        return {"type": "text", "content": label or "[应用消息]"}

    if decoded.get("kind") == "image":
        return {"type": "image", "content": "", "fileName": "微信图片"}

    return {"type": "text", "content": text or f"[消息 {base}]"}


def _as_text(blob: bytes) -> str:
    for enc in ("utf-8", "utf-16-le", "gb18030"):
        try:
            return blob.decode(enc)
        except UnicodeDecodeError:
            continue
    return blob.decode("utf-8", "replace")


def _to_int(value) -> int | None:
    try:
        return int(str(value).strip())
    except (TypeError, ValueError):
        return None


def _load_contacts(con: sqlite3.Connection) -> dict[str, str]:
    names: dict[str, str] = {}
    for username, nick, remark in con.execute(
            "SELECT username, nick_name, remark FROM contact"):
        display = (remark or "").strip() or (nick or "").strip() or username
        names[username] = display
    return names


def _load_avatar_urls(con: sqlite3.Connection) -> list[dict]:
    """联系人表里每个联系人都带头像 CDN 地址（群成员也有）。

    head_image.db 只缓存了部分人的头像，群成员往往不在里面；用联系人表的
    地址按需下载即可补齐，不必把图片预先存进本地库。
    """
    out: list[dict] = []
    seen: set[str] = set()
    for username, small, big, md5 in con.execute(
            "SELECT username, small_head_url, big_head_url, head_img_md5 FROM contact"):
        name = (username or "").strip()
        if not name or name in seen:
            continue
        url = (small or "").strip() or (big or "").strip()
        if not url:
            digest = (md5 or "").strip().lower()
            if len(digest) == 32 and all(ch in "0123456789abcdef" for ch in digest):
                url = f"https://wx.qlogo.cn/mmhead/{digest}/132"
        if not url.startswith("https://"):
            continue
        seen.add(name)
        out.append({"entityId": name, "url": url})
    return out


def _load_rooms(con: sqlite3.Connection, contacts: dict[str, str]) -> list[dict]:
    member_counts: dict[int, int] = {}
    for room_id, count in con.execute(
            "SELECT room_id, COUNT(*) FROM chatroom_member GROUP BY room_id"):
        member_counts[int(room_id)] = int(count)

    rooms = []
    for room_id, username in con.execute("SELECT id, username FROM chat_room"):
        rooms.append({
            "id": username,
            "name": contacts.get(username) or username,
            "memberCount": member_counts.get(int(room_id), 0),
            "kind": "group",
        })
    return rooms


# 会话表里混着系统号、公众号、文件传输助手等，这些不是真实联系人。
SPECIAL_SESSIONS = {
    "filehelper", "weixin", "newsapp", "fmessage", "medianote", "floatbottle",
    "qqmail", "tmessage", "qmessage", "officialaccounts",
    "notification_messages", "weixinremind", "brandsessionholder",
    "brandservicesessionholder", "exmail_tool", "notifymessage",
}


def _load_private_rooms(
    sessions: dict[str, int],
    contacts: dict[str, str],
    available_tables: set[str],
) -> list[dict]:
    """从会话表补出真实联系人的单聊，排除系统号和公众号。

    只保留消息库里存在消息表的会话，避免把从没聊过的联系人塞进列表。
    """
    rooms = []
    for username in sessions:
        if not username or username.endswith("@chatroom"):
            continue
        if username.lower() in SPECIAL_SESSIONS or username.startswith("gh_"):
            continue
        name = contacts.get(username)
        if not name or _msg_table(username) not in available_tables:
            continue
        rooms.append({
            "id": username,
            "name": name,
            "memberCount": 2,
            "kind": "private",
        })
    return rooms


def _load_sessions(con: sqlite3.Connection) -> dict[str, int]:
    out: dict[str, int] = {}
    for username, ts in con.execute(
            "SELECT username, last_timestamp FROM SessionTable"):
        try:
            out[username] = int(ts or 0)
        except (TypeError, ValueError):
            out[username] = 0
    return out


def _load_sender_ids(con: sqlite3.Connection) -> dict[int, str]:
    out: dict[int, str] = {}
    for rowid, username in con.execute("SELECT rowid, user_name FROM Name2Id"):
        out[int(rowid)] = username or ""
    return out


def _load_image_md5(
    storage: str, tmp_dir: str, keys: KeyRing
) -> dict[tuple[str, int], str]:
    """从 message_resource.db 建立 (群/联系人, local_id) -> 图片文件 md5 的映射。

    消息库里的图片 XML md5 并不是本地 .dat 文件名；真正对应磁盘文件的是
    资源库 packed_info 里的 32 位 md5。资源库读不到时降级为无图片预览。
    """
    src = os.path.join(storage, "message", "message_resource.db")
    if not os.path.isfile(src):
        return {}
    try:
        plain = os.path.join(tmp_dir, "message_resource.db")
        bad = _decrypt_to(src, plain, keys)
        if bad:
            log(f"消息资源库有 {bad} 页校验失败，已跳过图片索引")
            return {}
        con = _open_plain(plain)
        try:
            chats = {
                int(rowid): name
                for rowid, name in con.execute("SELECT rowid, user_name FROM ChatName2Id")
            }
            out: dict[tuple[str, int], str] = {}
            for chat_id, local_id, packed in con.execute(
                "SELECT chat_id, message_local_id, packed_info "
                "FROM MessageResourceInfo WHERE message_local_type = 3"
            ):
                if not packed:
                    continue
                match = re.search(rb"[0-9a-fA-F]{32}", bytes(packed))
                name = chats.get(int(chat_id))
                if match and name:
                    out[(name, int(local_id))] = match.group(0).decode().lower()
            return out
        finally:
            con.close()
    except Exception as exc:  # noqa: BLE001
        log(f"读取消息资源库失败，图片将只显示占位：{exc}")
        return {}


def _image_mime(blob: bytes) -> str | None:
    if blob[:3] == b"\xff\xd8\xff":
        return "image/jpeg"
    if blob[:8] == b"\x89PNG\r\n\x1a\n":
        return "image/png"
    if blob[:4] == b"RIFF" and blob[8:12] == b"WEBP":
        return "image/webp"
    if blob[:6] in (b"GIF87a", b"GIF89a"):
        return "image/gif"
    return None


def _load_avatars(storage: str, tmp_dir: str, keys: KeyRing) -> list[dict]:
    """读取头像库（与消息库同一口令），返回可直接展示的头像列表。

    头像库属于缓存数据，缺失、版本差异或个别页校验失败都不应阻断采集，
    因此这里整体降级为「没有头像」，由前端回退到姓名首字占位。
    """
    src = os.path.join(storage, "head_image", "head_image.db")
    if not os.path.isfile(src):
        return []
    try:
        plain = os.path.join(tmp_dir, "head_image.db")
        bad = _decrypt_to(src, plain, keys)
        if bad:
            log(f"头像库有 {bad} 页校验失败，已跳过头像")
            return []
        con = _open_plain(plain)
        try:
            rows = con.execute("SELECT username, image_buffer FROM head_image").fetchall()
        finally:
            con.close()
    except Exception as exc:  # noqa: BLE001
        log(f"读取头像库失败，已跳过头像：{exc}")
        return []

    avatars: list[dict] = []
    for username, blob in rows:
        image = blob or b""
        mime = _image_mime(image)
        if not username or not mime or len(image) > 512 * 1024:
            continue
        avatars.append({
            "entityId": username,
            "mime": mime,
            "data": base64.b64encode(image).decode("ascii"),
        })
    return avatars


def _load_contact_avatar_urls(
    storage: str, tmp_dir: str, keys: KeyRing
) -> list[dict]:
    """只解联系人库，取出头像 CDN 地址（群成员也能覆盖到）。"""
    src = os.path.join(storage, "contact", "contact.db")
    if not os.path.isfile(src):
        return []
    try:
        plain = os.path.join(tmp_dir, "contact.db")
        bad = _decrypt_to(src, plain, keys)
        if bad:
            log(f"联系人库有 {bad} 页校验失败，已跳过头像地址")
            return []
        con = _open_plain(plain)
        try:
            return _load_avatar_urls(con)
        finally:
            con.close()
    except Exception as exc:  # noqa: BLE001
        log(f"读取联系人头像地址失败：{exc}")
        return []


def _msg_table(username: str) -> str:
    return "Msg_" + hashlib.md5(username.encode("utf-8")).hexdigest()


def _collect_messages(con: sqlite3.Connection, rooms: list[dict],
                      sender_ids: dict[int, str], contacts: dict[str, str],
                      since: dict[str, int],
                      image_md5: dict[tuple[str, int], str] | None = None) -> tuple[list[dict], dict[str, int]]:
    tables = set(_tables(con))
    messages: list[dict] = []
    active: dict[str, int] = {}
    for room in rooms:
        table = _msg_table(room["id"])
        if table not in tables:
            continue
        room_id = room["id"]
        floor = int(since.get(room_id, 0) or 0)
        rows = con.execute(
            f'SELECT local_id, local_type, create_time, real_sender_id, message_content '
            f'FROM "{table}" WHERE create_time > ? ORDER BY create_time DESC LIMIT ?',
            (floor, MAX_MESSAGES_PER_GROUP),
        ).fetchall()
        count = 0
        for local_id, local_type, created_at, sender_ref, raw in rows:
            try:
                local_type = int(local_type)
            except (TypeError, ValueError):
                continue
            if _to_int(created_at) is None:
                continue
            sender = sender_ids.get(int(sender_ref or 0), "")
            sender_name = contacts.get(sender) or sender or "未知成员"
            try:
                decoded = _decode_message(raw, local_type, sender)
            except Exception:
                decoded = {"type": "text", "content": "[无法解析的消息]"}
            if decoded.get("type") == "skip":
                continue
            ts = int(created_at)
            active[room_id] = max(active.get(room_id, 0), ts)
            messages.append({
                "groupId": room_id,
                "localId": int(local_id or 0),
                "senderId": sender,
                "senderName": sender_name,
                "type": decoded["type"],
                "content": decoded.get("content") or "",
                "quotedSender": decoded.get("quotedSender"),
                "quotedContent": decoded.get("quotedContent"),
                "fileName": decoded.get("fileName"),
                "fileSize": decoded.get("fileSize"),
                "imageMd5": (image_md5 or {}).get((room_id, int(local_id or 0)))
                if decoded["type"] == "image" else None,
                "emoticonMd5": decoded.get("emoticonMd5"),
                "emoticonUrl": decoded.get("emoticonUrl"),
                "emoticonKey": decoded.get("emoticonKey"),
                "createdAt": ts,
            })
            count += 1
            if count >= MAX_MESSAGES_PER_GROUP:
                break
    return messages, active


def export(request: dict) -> dict:
    db_root = resolve_db_root(request)
    since = request.get("since") or {}
    keys = KeyRing(request.get("keyHex") or "")

    account = request.get("account")
    if not account or not _key_ok(
            os.path.join(db_root, account, "db_storage", "contact", "contact.db"), keys):
        account = _pick_account(db_root, keys)
    if not account:
        raise RuntimeError("未找到可用密钥解开的微信账号目录（密钥可能已失效）")

    storage = os.path.join(db_root, account, "db_storage")
    msg_dir = os.path.join(storage, "message")
    if not os.path.isdir(msg_dir):
        raise RuntimeError("微信数据目录结构不符合预期（缺少 message 目录）")

    tmp_dir = tempfile.mkdtemp(prefix="jy-wechat-")
    try:
        plain_contact = os.path.join(tmp_dir, "contact.db")
        plain_session = os.path.join(tmp_dir, "session.db")
        bad = _decrypt_to(os.path.join(storage, "contact", "contact.db"), plain_contact, keys)
        if bad:
            raise RuntimeError("联系人库校验失败（密钥与库不匹配）")
        _decrypt_to(os.path.join(storage, "session", "session.db"), plain_session, keys)

        contact_con = _open_plain(plain_contact)
        contacts = _load_contacts(contact_con)
        rooms = _load_rooms(contact_con, contacts)
        avatar_urls = _load_avatar_urls(contact_con)
        contact_con.close()

        session_con = _open_plain(plain_session)
        sessions = _load_sessions(session_con)
        session_con.close()

        all_messages: list[dict] = []
        active: dict[str, int] = {}
        image_md5 = _load_image_md5(storage, tmp_dir, keys)
        shards = sorted(
            name for name in os.listdir(msg_dir)
            if re.fullmatch(r"message_\d+\.db", name)
        )
        if not shards:
            raise RuntimeError("未找到消息库分片（message_*.db）")

        plain_shards: list[str] = []
        for shard in shards:
            plain_shard = os.path.join(tmp_dir, shard)
            bad = _decrypt_to(os.path.join(msg_dir, shard), plain_shard, keys)
            if bad:
                log(f"{shard} 有 {bad} 页校验失败，已跳过")
            plain_shards.append(plain_shard)

        # 先汇总所有分片的消息表，用于判断单聊会话是否真的有消息可采。
        available_tables: set[str] = set()
        for plain_shard in plain_shards:
            shard_con = _open_plain(plain_shard)
            try:
                available_tables |= set(_tables(shard_con))
            finally:
                shard_con.close()

        rooms.extend(_load_private_rooms(sessions, contacts, available_tables))

        for plain_shard in plain_shards:
            shard_con = _open_plain(plain_shard)
            try:
                sender_ids = _load_sender_ids(shard_con)
                msgs, act = _collect_messages(shard_con, rooms, sender_ids,
                                              contacts, since, image_md5)
            finally:
                shard_con.close()
            all_messages.extend(msgs)
            for room_id, ts in act.items():
                active[room_id] = max(active.get(room_id, 0), ts)

        for room in rooms:
            room["lastActive"] = max(
                int(sessions.get(room["id"], 0) or 0),
                active.get(room["id"], 0),
            )

        rooms.sort(key=lambda r: r["lastActive"], reverse=True)
        avatars = _load_avatars(storage, tmp_dir, keys)
        return {
            "account": account,
            "groups": rooms,
            "messages": all_messages,
            "avatars": avatars,
            "avatarUrls": avatar_urls,
        }
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


def probe_status(request: dict) -> dict:
    """安装向导检测：列出账号目录，并校验 contact.db（不解消息库、不写库）。"""
    db_root = resolve_db_root(request)
    accounts: list[str] = []
    if os.path.isdir(db_root):
        for entry in sorted(os.listdir(db_root)):
            contact = os.path.join(db_root, entry, "db_storage", "contact", "contact.db")
            if os.path.isfile(contact):
                accounts.append(entry)

    result = {
        "accounts": accounts,
        "account": "",
        "keyPresent": False,
        "dbRoot": db_root,
        "message": "",
    }
    if not accounts:
        result["message"] = "还没有找到本机微信聊天记录，请先登录微信"
        return result

    key_hex = (request.get("keyHex") or "").strip().lower().removeprefix("0x")
    if len(key_hex) == 64:
        keys = KeyRing(key_hex)
        result["keyPresent"] = True
    elif sys.platform == "win32":
        keys = KeyRing("")
        try:
            keys.ensure()
        except Exception as exc:  # noqa: BLE001 - 把可操作的原因交给界面
            result["message"] = str(exc)
            return result
        result["keyPresent"] = True
    else:
        result["message"] = "还没有读取授权"
        return result

    account = request.get("account")
    if account and _key_ok(
            os.path.join(db_root, account, "db_storage", "contact", "contact.db"), keys):
        result["account"] = account
    else:
        result["account"] = _pick_account(db_root, keys) or ""
    result["message"] = (
        "已找到微信数据，可以开始同步" if result["account"]
        else "找到了微信数据，但无法打开数据库，请重试"
    )
    return result


def probe_sessions(request: dict) -> dict:
    """只解密 session.db，返回关注会话的最后消息时间（不读消息库、不写库）。"""
    db_root = resolve_db_root(request)
    keys = KeyRing(request.get("keyHex") or "")

    account = request.get("account")
    if not account or not _key_ok(
            os.path.join(db_root, account, "db_storage", "contact", "contact.db"), keys):
        account = _pick_account(db_root, keys)
    if not account:
        raise RuntimeError("未找到可用密钥解开的微信账号目录（密钥可能已失效）")

    session_db = os.path.join(db_root, account, "db_storage", "session", "session.db")
    if not os.path.isfile(session_db):
        raise RuntimeError("未找到会话库")

    wanted = set(request.get("usernames") or [])
    tmp_dir = tempfile.mkdtemp(prefix="jy-session-")
    try:
        plain = os.path.join(tmp_dir, "session.db")
        bad = _decrypt_to(session_db, plain, keys)
        if bad:
            raise RuntimeError("会话库校验失败（密钥与库不匹配）")
        con = _open_plain(plain)
        try:
            stamps: dict[str, int] = {}
            for username, ts in con.execute(
                    "SELECT username, last_timestamp FROM SessionTable"):
                if not wanted or username in wanted:
                    stamps[username] = int(ts or 0)
        finally:
            con.close()
        return {"account": account, "lastTimestamps": stamps}
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


def probe_avatars(request: dict) -> dict:
    """只解密头像库，返回可展示的头像（不读消息库、不写库）。"""
    db_root = resolve_db_root(request)
    keys = KeyRing(request.get("keyHex") or "")

    account = request.get("account")
    if not account or not _key_ok(
            os.path.join(db_root, account, "db_storage", "contact", "contact.db"), keys):
        account = _pick_account(db_root, keys)
    if not account:
        raise RuntimeError("未找到可用密钥解开的微信账号目录（密钥可能已失效）")

    storage = os.path.join(db_root, account, "db_storage")
    tmp_dir = tempfile.mkdtemp(prefix="jy-avatars-")
    try:
        return {
            "account": account,
            "avatars": _load_avatars(storage, tmp_dir, keys),
            "avatarUrls": _load_contact_avatar_urls(storage, tmp_dir, keys),
        }
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


def main() -> int:
    try:
        raw = sys.stdin.read()
        request = json.loads(raw) if raw.strip() else {}
        probe = request.get("probe")
        if probe == "status":
            result = probe_status(request)
        elif probe == "session":
            result = probe_sessions(request)
        elif probe == "avatars":
            result = probe_avatars(request)
        else:
            result = export(request)
        json.dump(result, sys.stdout, ensure_ascii=False)
        sys.stdout.write("\n")
        sys.stdout.flush()
        return 0
    except Exception as exc:  # noqa: BLE001
        log(f"采集失败：{exc}")
        return 2


if __name__ == "__main__":
    sys.exit(main())
