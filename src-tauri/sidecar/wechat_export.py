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


def _decrypt_to(db_path: str, out_path: str, passphrase: bytes) -> int:
    """解密整库到临时文件，返回 HMAC 校验失败的页数。"""
    salt = sc.read_salt(db_path)
    enc_key, hmac_key = _derive(passphrase, salt)
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


def _key_ok(db_path: str, passphrase: bytes) -> bool:
    try:
        salt = sc.read_salt(db_path)
        _, hmac_key = _derive(passphrase, salt)
        with open(db_path, "rb") as f:
            page = f.read(PAGE)
        return len(page) == PAGE and sc.verify_page(page, 1, hmac_key)
    except Exception:
        return False


def _pick_account(db_root: str, passphrase: bytes) -> str | None:
    if not os.path.isdir(db_root):
        return None
    for entry in sorted(os.listdir(db_root)):
        contact = os.path.join(db_root, entry, "db_storage", "contact", "contact.db")
        if os.path.isfile(contact) and _key_ok(contact, passphrase):
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
        return {"type": "text", "content": "[表情]"}
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


def _load_avatars(storage: str, tmp_dir: str, passphrase: bytes) -> list[dict]:
    """读取头像库（与消息库同一口令），返回可直接展示的头像列表。

    头像库属于缓存数据，缺失、版本差异或个别页校验失败都不应阻断采集，
    因此这里整体降级为「没有头像」，由前端回退到姓名首字占位。
    """
    src = os.path.join(storage, "head_image", "head_image.db")
    if not os.path.isfile(src):
        return []
    try:
        plain = os.path.join(tmp_dir, "head_image.db")
        bad = _decrypt_to(src, plain, passphrase)
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


def _msg_table(username: str) -> str:
    return "Msg_" + hashlib.md5(username.encode("utf-8")).hexdigest()


def _collect_messages(con: sqlite3.Connection, rooms: list[dict],
                      sender_ids: dict[int, str], contacts: dict[str, str],
                      since: dict[str, int]) -> tuple[list[dict], dict[str, int]]:
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
                "createdAt": ts,
            })
            count += 1
            if count >= MAX_MESSAGES_PER_GROUP:
                break
    return messages, active


def export(request: dict) -> dict:
    key_hex = (request.get("keyHex") or "").strip().lower().removeprefix("0x")
    if len(key_hex) != 64:
        raise RuntimeError("密钥格式不正确（需要 32 字节口令）")
    passphrase = bytes.fromhex(key_hex)
    db_root = request.get("dbRoot") or DEFAULT_DB_ROOT
    since = request.get("since") or {}

    account = request.get("account")
    if not account or not _key_ok(
            os.path.join(db_root, account, "db_storage", "contact", "contact.db"), passphrase):
        account = _pick_account(db_root, passphrase)
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
        bad = _decrypt_to(os.path.join(storage, "contact", "contact.db"), plain_contact, passphrase)
        if bad:
            raise RuntimeError("联系人库校验失败（密钥与库不匹配）")
        _decrypt_to(os.path.join(storage, "session", "session.db"), plain_session, passphrase)

        contact_con = _open_plain(plain_contact)
        contacts = _load_contacts(contact_con)
        rooms = _load_rooms(contact_con, contacts)
        contact_con.close()

        session_con = _open_plain(plain_session)
        sessions = _load_sessions(session_con)
        session_con.close()

        all_messages: list[dict] = []
        active: dict[str, int] = {}
        shards = sorted(
            name for name in os.listdir(msg_dir)
            if re.fullmatch(r"message_\d+\.db", name)
        )
        if not shards:
            raise RuntimeError("未找到消息库分片（message_*.db）")
        for shard in shards:
            plain_shard = os.path.join(tmp_dir, shard)
            bad = _decrypt_to(os.path.join(msg_dir, shard), plain_shard, passphrase)
            if bad:
                log(f"{shard} 有 {bad} 页校验失败，已跳过")
            shard_con = _open_plain(plain_shard)
            try:
                sender_ids = _load_sender_ids(shard_con)
                msgs, act = _collect_messages(shard_con, rooms, sender_ids,
                                              contacts, since)
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
        avatars = _load_avatars(storage, tmp_dir, passphrase)
        return {
            "account": account,
            "groups": rooms,
            "messages": all_messages,
            "avatars": avatars,
        }
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


def probe_sessions(request: dict) -> dict:
    """只解密 session.db，返回关注会话的最后消息时间（不读消息库、不写库）。"""
    key_hex = (request.get("keyHex") or "").strip().lower().removeprefix("0x")
    if len(key_hex) != 64:
        raise RuntimeError("密钥格式不正确（需要 32 字节口令）")
    passphrase = bytes.fromhex(key_hex)
    db_root = request.get("dbRoot") or DEFAULT_DB_ROOT

    account = request.get("account")
    if not account or not _key_ok(
            os.path.join(db_root, account, "db_storage", "contact", "contact.db"), passphrase):
        account = _pick_account(db_root, passphrase)
    if not account:
        raise RuntimeError("未找到可用密钥解开的微信账号目录（密钥可能已失效）")

    session_db = os.path.join(db_root, account, "db_storage", "session", "session.db")
    if not os.path.isfile(session_db):
        raise RuntimeError("未找到会话库")

    wanted = set(request.get("usernames") or [])
    tmp_dir = tempfile.mkdtemp(prefix="jy-session-")
    try:
        plain = os.path.join(tmp_dir, "session.db")
        bad = _decrypt_to(session_db, plain, passphrase)
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
    key_hex = (request.get("keyHex") or "").strip().lower().removeprefix("0x")
    if len(key_hex) != 64:
        raise RuntimeError("密钥格式不正确（需要 32 字节口令）")
    passphrase = bytes.fromhex(key_hex)
    db_root = request.get("dbRoot") or DEFAULT_DB_ROOT

    account = request.get("account")
    if not account or not _key_ok(
            os.path.join(db_root, account, "db_storage", "contact", "contact.db"), passphrase):
        account = _pick_account(db_root, passphrase)
    if not account:
        raise RuntimeError("未找到可用密钥解开的微信账号目录（密钥可能已失效）")

    storage = os.path.join(db_root, account, "db_storage")
    tmp_dir = tempfile.mkdtemp(prefix="jy-avatars-")
    try:
        return {
            "account": account,
            "avatars": _load_avatars(storage, tmp_dir, passphrase),
        }
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


def main() -> int:
    try:
        raw = sys.stdin.read()
        request = json.loads(raw) if raw.strip() else {}
        probe = request.get("probe")
        if probe == "session":
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
