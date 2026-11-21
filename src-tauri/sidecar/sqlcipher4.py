"""SQLCipher 4 页格式（微信 4.x 实测参数），纯标准库实现。

实测/核对来源：
- SQLCipher 官方设计文档：AES-256-CBC、每页独立随机 IV 存于页尾、HMAC-SHA512、
  PBKDF2-HMAC-SHA512 默认 256000 次、文件前 16 字节为 salt。
- SQLCipher 源码 src/sqlcipher.c: sqlcipher_codec_ctx_reserve_setup()
  reserve = iv_sz(16) + hmac_sz(64) = 80；sqlcipher_page_hmac() 的 HMAC 输入为
  「密文 + IV + pgno(4 字节小端)」；sqlcipher_page_cipher() 的 IV 位于 page_size-reserve 处。
- 父会话从 Windows 侧核实的微信参数：page_size=4096, reserve=80, HMAC-SHA512, kdf_iter=256000。

与平台无关，可直接用于 Windows 产出的同名库。
"""

from __future__ import annotations

import hashlib
import hmac
import struct

from aes_pure import cbc_decrypt, cbc_encrypt

PAGE_SIZE = 4096
RESERVE = 80          # 16(IV) + 64(HMAC-SHA512)
IV_SIZE = 16
HMAC_SIZE = 64        # SHA-512
KEY_SIZE = 32
KDF_ITER = 256000
HMAC_SALT_MASK = 0x3a
FAST_KDF_ITER = 2     # HMAC 子密钥用 2 次迭代
FILE_HEADER_SZ = 16   # 文件头 = salt 长度；第 1 页从这里开始加密
SQLITE_FILE_HEADER = b"SQLite format 3\x00"


def derive_keys(key_material: bytes, salt: bytes,
                kdf_iter: int = KDF_ITER) -> tuple[bytes, bytes]:
    """返回 (加密密钥, HMAC 密钥)。

    key_material 为 32 字节时按「原始密钥」处理（微信从内存取到的就是 32 字节原始密钥，
    等价于 SQLCipher 的 PRAGMA key = "x'...'"），否则按口令走 PBKDF2。
    """
    if len(salt) != IV_SIZE:
        raise ValueError("salt 必须是 16 字节")
    if len(key_material) == KEY_SIZE:
        enc_key = key_material
    else:
        enc_key = hashlib.pbkdf2_hmac("sha512", key_material, salt, kdf_iter, KEY_SIZE)
    hmac_salt = bytes(b ^ HMAC_SALT_MASK for b in salt)
    hmac_key = hashlib.pbkdf2_hmac("sha512", enc_key, hmac_salt, FAST_KDF_ITER, KEY_SIZE)
    return enc_key, hmac_key


def page_hmac(hmac_key: bytes, page: bytes, pgno: int) -> bytes:
    """HMAC 覆盖密文 + IV，再追加 4 字节小端页号。

    第 1 页从偏移 16 开始（前 16 字节是明文 salt，不参与加密与校验），
    与 SQLCipher sqlite3Codec() 的 offset=FILE_HEADER_SZ 行为一致。
    """
    off = FILE_HEADER_SZ if pgno == 1 else 0
    usable_end = PAGE_SIZE - RESERVE
    mac = hmac.new(hmac_key, page[off:usable_end + IV_SIZE], hashlib.sha512)
    mac.update(struct.pack("<I", pgno))
    return mac.digest()


def verify_page(page: bytes, pgno: int, hmac_key: bytes) -> bool:
    usable_end = PAGE_SIZE - RESERVE
    stored = page[usable_end + IV_SIZE:usable_end + IV_SIZE + HMAC_SIZE]
    return hmac.compare_digest(stored, page_hmac(hmac_key, page, pgno))


def decrypt_page(page: bytes, pgno: int, enc_key: bytes) -> bytes:
    """解密单页，返回完整 4096 字节明文页（预留区补零）。"""
    off = FILE_HEADER_SZ if pgno == 1 else 0
    usable_end = PAGE_SIZE - RESERVE
    iv = page[usable_end:usable_end + IV_SIZE]
    plain = bytearray(cbc_decrypt(enc_key, iv, page[off:usable_end]))
    if pgno == 1:
        # 文件头 16 字节在 SQLCipher 中不落盘为明文，内存页里是 SQLite 魔数
        return SQLITE_FILE_HEADER + bytes(plain) + bytes(RESERVE)
    return bytes(plain) + bytes(RESERVE)


def encrypt_page(plain: bytes, pgno: int, enc_key: bytes, hmac_key: bytes,
                 iv: bytes, salt: bytes | None = None) -> bytes:
    """加密单页：plain 为完整 4096 字节明文页，返回 4096 字节密文页。

    第 1 页的前 16 字节在文件中是 salt（明文），SQLite 魔数只存在于解密后的内存页。
    """
    if len(plain) != PAGE_SIZE:
        raise ValueError(f"明文页应为 {PAGE_SIZE} 字节，实际 {len(plain)}")
    off = FILE_HEADER_SZ if pgno == 1 else 0
    if pgno == 1 and (salt is None or len(salt) != IV_SIZE):
        raise ValueError("加密第 1 页必须提供 16 字节 salt")
    usable_end = PAGE_SIZE - RESERVE
    body = cbc_encrypt(enc_key, iv, plain[off:usable_end])
    page = bytearray(PAGE_SIZE)
    if pgno == 1:
        page[0:off] = salt
    page[off:usable_end] = body
    page[usable_end:usable_end + IV_SIZE] = iv
    page[usable_end + IV_SIZE:usable_end + IV_SIZE + HMAC_SIZE] = page_hmac(
        hmac_key, bytes(page), pgno)
    return bytes(page)


def file_pages(path: str) -> int:
    import os
    return os.path.getsize(path) // PAGE_SIZE


def read_salt(path: str) -> bytes:
    with open(path, "rb") as f:
        return f.read(IV_SIZE)


def is_plaintext_sqlite(head: bytes) -> bool:
    return head.startswith(b"SQLite format 3\x00")
