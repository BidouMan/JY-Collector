"""微信消息内容解码（与平台无关）。

重要说明：
- `local_type` 映射表沿用公开的微信 3.x/4.x 通用类型编号，4.2.0.17 未逐项实测，
  输出中一律标注 decode_note，未验证的路径明确写"待核实"。
- 4.x 部分字段为 protobuf 编码，本文件用「线格式扫描」做启发式提取，
  不假设固定 schema，避免臆测字段号。
"""

from __future__ import annotations

import re
import zlib

# 公开资料中的消息类型编号（微信 3.x 通用，4.x 待逐项核实）
LOCAL_TYPE_NAMES = {
    1: "文本",
    3: "图片",
    34: "语音",
    42: "名片",
    43: "视频",
    47: "表情",
    48: "位置",
    49: "应用消息(链接/文件/引用/小程序)",
    50: "通话",
    10000: "系统消息",
    10002: "系统消息/撤回",
}

ZSTD_MAGIC = b"\x28\xb5\x2f\xfd"


def local_type_name(t) -> str:
    try:
        t = int(t)
    except (TypeError, ValueError):
        return f"未知({t})"
    return LOCAL_TYPE_NAMES.get(t, f"其他({t})")


def _maybe_zstd(blob: bytes) -> bytes | None:
    """Python 3.14+ 内置 compression.zstd；失败则返回 None。"""
    if not blob.startswith(ZSTD_MAGIC):
        return None
    try:
        from compression import zstd  # type: ignore
        return zstd.decompress(blob)
    except Exception:
        return None


def _text_of(blob: bytes) -> str:
    for enc in ("utf-8", "utf-16-le", "gb18030"):
        try:
            return blob.decode(enc)
        except UnicodeDecodeError:
            continue
    return blob.decode("utf-8", "replace")


def _xml_field(xml: str, tag: str) -> str | None:
    m = re.search(rf"<{tag}>(.*?)</{tag}>", xml, re.S)
    if not m:
        return None
    val = m.group(1)
    val = re.sub(r"<!\[CDATA\[(.*?)\]\]>", r"\1", val, flags=re.S)
    return val.strip()


def scan_proto_strings(blob: bytes, min_len: int = 2) -> list[str]:
    """protobuf 线格式扫描：提取所有可打印的 length-delimited 字段。

    不依赖字段号，只用于在 schema 未知时把可读文本捞出来（启发式，待核实）。
    """
    out: list[str] = []
    i = 0
    n = len(blob)
    while i < n:
        key = blob[i]
        i += 1
        field_num, wire = key >> 3, key & 0x07
        if field_num == 0:
            break
        if wire == 0:          # varint
            while i < n and blob[i] & 0x80:
                i += 1
            i += 1
        elif wire == 1:        # 64-bit
            i += 8
        elif wire == 2:        # length-delimited
            ln = 0
            shift = 0
            while i < n:
                b = blob[i]
                i += 1
                ln |= (b & 0x7F) << shift
                shift += 7
                if not b & 0x80:
                    break
            if ln <= 0 or i + ln > n:
                break
            payload = blob[i:i + ln]
            i += ln
            try:
                s = payload.decode("utf-8")
                if s.isprintable() and len(s) >= min_len:
                    out.append(s)
                    continue
            except UnicodeDecodeError:
                pass
            out.extend(scan_proto_strings(payload, min_len))
        elif wire == 5:        # 32-bit
            i += 4
        else:
            break
    return out


def decode_content(raw, local_type) -> dict:
    """把 message_content 解成可读结构。"""
    if raw is None:
        return {"kind": "empty", "text": None, "decode_note": "内容为空"}
    if isinstance(raw, str):
        raw = raw.encode("utf-8", "surrogateescape")

    inflated = _maybe_zstd(raw)
    if inflated is not None:
        raw = inflated
        method = "zstd+"
    else:
        method = ""

    text = _text_of(raw)
    head = text.lstrip()[:1]

    if head == "<":
        method += "xml"
        res = {
            "kind": "xml",
            "text": _xml_field(text, "title") or _xml_field(text, "content") or text[:200],
            "app_type": _xml_field(text, "type"),
            "des": _xml_field(text, "des"),
            "url": _xml_field(text, "url"),
            "refer": None,
            "decode_note": "XML 结构直接解析",
        }
        refer_block = re.search(r"<refermsg>(.*?)</refermsg>", text, re.S)
        if refer_block:
            rb = refer_block.group(1)
            res["refer"] = {
                "displayname": _xml_field(rb, "displayname"),
                "content": _xml_field(rb, "content"),
                "svrid": _xml_field(rb, "svrid"),
                "type": _xml_field(rb, "type"),
            }
            res["kind"] = "quote"
        img = re.search(r"<img[^>]*>", text)
        if img and not res.get("refer"):
            attrs = dict(re.findall(r'(\w+)="([^"]*)"', img.group(0)))
            res["kind"] = "image"
            res["image"] = {k: attrs[k] for k in
                            ("md5", "length", "cdnthumburl", "cdnbigimgurl", "aeskey")
                            if k in attrs}
        res["method"] = method
        res["text"] = res["text"][:300] if res["text"] else None
        return res

    try:
        clean = text
        if all(ch.isprintable() or ch in "\r\n\t" for ch in clean):
            return {"kind": "text", "text": clean, "method": method + "utf8",
                    "decode_note": "纯文本直接解码"}
    except Exception:
        pass

    strings = scan_proto_strings(raw)
    return {
        "kind": "protobuf-ish",
        "text": strings[0] if strings else None,
        "strings": strings[:8],
        "method": method + "proto-scan",
        "decode_note": "schema 未明，按线格式启发式提取，待核实",
    }
