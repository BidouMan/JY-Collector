"""SQLite WAL 解析与合并（含 salt 过滤 + 校验和验证）。

微信 4.x 的 -wal 为预分配/多世代文件，checkpoint 后仍残留旧世代帧。
旧世代帧的 salt 与当前 WAL 头不一致，若不按 salt 过滤直接合并，
会用过期页覆盖当前页，导致 "database malformed"。
"""

import struct

WAL_HDR_SIZE = 32
FRAME_HDR_SIZE = 24


def _checksum(data: bytes, s1: int, s2: int, big_endian: bool) -> tuple[int, int]:
    fmt = ">II" if big_endian else "<II"
    for i in range(0, len(data) - 7, 8):
        x, y = struct.unpack_from(fmt, data, i)
        s1 = (s1 + x + s2) & 0xFFFFFFFF
        s2 = (s2 + y + s1) & 0xFFFFFFFF
    return s1, s2


def _detect_byte_order(blob: bytes, c1: int, c2: int) -> tuple[bool, bool]:
    """返回 (big_endian, 头校验和是否自洽)。

    SQLite 规范里 magic 的末位标识校验和字节序，但实测微信 4.2.0.17 的 -wal
    为 magic=0x377f0682 却使用小端字（见 outputs/REPORT.md），因此这里不按 magic
    推断，而是两种字节序都试一遍，取能对上的那一种；都对不上则记录为不自洽。
    """
    for big in (True, False):
        s1, s2 = _checksum(blob[0:24], 0, 0, big)
        if (s1, s2) == (c1, c2):
            return big, True
    return False, False


def parse_wal(path: str) -> dict:
    """解析 WAL，返回头信息、帧列表（含 salt/校验和是否有效）。"""
    with open(path, "rb") as f:
        blob = f.read()
    if len(blob) < WAL_HDR_SIZE:
        return {"ok": False, "reason": "文件小于 32 字节，非 WAL"}

    magic, version, page_size, ckpt_seq, salt1, salt2, c1, c2 = struct.unpack_from(
        ">IIIIIIII", blob, 0)
    if magic not in (0x377F0682, 0x377F0683):
        return {"ok": False, "reason": f"magic 不匹配: 0x{magic:08x}"}

    big_endian, header_ck_ok = _detect_byte_order(blob, c1, c2)
    if page_size < 512 or page_size > 65536:
        return {"ok": False, "reason": f"页大小异常: {page_size}"}

    frame_sz = FRAME_HDR_SIZE + page_size
    n_frames = (len(blob) - WAL_HDR_SIZE) // frame_sz if frame_sz else 0

    frames = []
    s1, s2 = (c1, c2) if header_ck_ok else (0, 0)
    for i in range(n_frames):
        off = WAL_HDR_SIZE + i * frame_sz
        pgno, db_size, fs1, fs2, fc1, fc2 = struct.unpack_from(">IIIIII", blob, off)
        page = blob[off + FRAME_HDR_SIZE: off + frame_sz]

        s1, s2 = _checksum(blob[off:off + 8], s1, s2, big_endian)
        s1, s2 = _checksum(page, s1, s2, big_endian)

        salt_match = (fs1 == salt1 and fs2 == salt2)
        ck_ok = (s1 == fc1 and s2 == fc2)
        frames.append({
            "index": i,
            "pgno": pgno,
            "db_size": db_size,
            "salt_match": salt_match,
            "checksum_ok": ck_ok,
            "commit": db_size != 0,
        })

    valid = [fr for fr in frames if fr["salt_match"] and fr["checksum_ok"]]
    stale = [fr for fr in frames if not fr["salt_match"]]
    return {
        "ok": True,
        "magic": f"0x{magic:08x}",
        "byte_order": "big" if big_endian else "little",
        "version": version,
        "page_size": page_size,
        "checkpoint_seq": ckpt_seq,
        "salt": (salt1, salt2),
        "header_checksum_ok": header_ck_ok,
        "frame_count": n_frames,
        "valid_frames": len(valid),
        "stale_salt_frames": len(stale),
        "checksum_bad_frames": len([fr for fr in frames if not fr["checksum_ok"]]),
        "last_commit_index": max([fr["index"] for fr in valid if fr["commit"]], default=None),
        "valid_commit_db_size": ([fr["db_size"] for fr in valid if fr["commit"]] or [None])[-1],
    }


def read_wal_pages(path: str, filter_salt: bool = True) -> tuple[dict, dict[int, bytes]]:
    """返回 (头信息, {pgno: 页字节})。

    filter_salt=True 时只取 salt 匹配且校验和正确的帧，并只应用到最后一个
    commit 帧为止（与 SQLite 恢复语义一致）。
    """
    info = parse_wal(path)
    if not info.get("ok"):
        return info, {}
    with open(path, "rb") as f:
        blob = f.read()
    page_size = info["page_size"]
    frame_sz = FRAME_HDR_SIZE + page_size
    pages: dict[int, bytes] = {}
    limit = info["last_commit_index"] if (filter_salt and info["last_commit_index"] is not None) else 10 ** 9
    for i in range(info["frame_count"]):
        off = WAL_HDR_SIZE + i * frame_sz
        pgno = struct.unpack_from(">I", blob, off)[0]
        if i > limit:
            break
        if filter_salt:
            fs1, fs2 = struct.unpack_from(">II", blob, off + 8)
            if (fs1, fs2) != info["salt"]:
                continue
        pages[pgno] = blob[off + FRAME_HDR_SIZE: off + frame_sz]
    return info, pages
