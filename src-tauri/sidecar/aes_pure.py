"""AES-256-CBC 最小实现（纯标准库，无外部依赖）。

只用于本 spike 的 SQLCipher 页解密/加密验证。
正确性由 test_pipeline.py 中的 NIST FIPS-197 测试向量保证。
"""

SBOX = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
]

INV_SBOX = [0] * 256
for _i, _v in enumerate(SBOX):
    INV_SBOX[_v] = _i

RCON = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36, 0x6c, 0xd8, 0xab, 0x4d]


def _xtime(a):
    a <<= 1
    if a & 0x100:
        a = (a ^ 0x1b) & 0xff
    return a


def _mul(a, b):
    r = 0
    for _ in range(8):
        if b & 1:
            r ^= a
        b >>= 1
        a = _xtime(a)
    return r


MUL2 = [_mul(i, 2) for i in range(256)]
MUL3 = [_mul(i, 3) for i in range(256)]
MUL9 = [_mul(i, 9) for i in range(256)]
MUL11 = [_mul(i, 11) for i in range(256)]
MUL13 = [_mul(i, 13) for i in range(256)]
MUL14 = [_mul(i, 14) for i in range(256)]


class AES256:
    """AES-256 单块加解密（ECB 核心），CBC 由 CBC 函数组合。"""

    rounds = 14

    def __init__(self, key: bytes):
        if len(key) != 32:
            raise ValueError("AES-256 需要 32 字节密钥")
        self.rk = self._expand_key(key)

    @staticmethod
    def _expand_key(key: bytes):
        w = [list(key[4 * i:4 * i + 4]) for i in range(8)]
        for i in range(8, 4 * (AES256.rounds + 1)):
            temp = list(w[i - 1])
            if i % 8 == 0:
                temp = temp[1:] + temp[:1]
                temp = [SBOX[b] for b in temp]
                temp[0] ^= RCON[i // 8 - 1]
            elif i % 8 == 4:
                temp = [SBOX[b] for b in temp]
            w.append([w[i - 8][j] ^ temp[j] for j in range(4)])
        return w

    def _add_round_key(self, s, rnd):
        rk = self.rk
        for c in range(4):
            k = rk[rnd * 4 + c]
            for r in range(4):
                s[r + 4 * c] ^= k[r]

    def encrypt_block(self, block: bytes) -> bytes:
        s = list(block)
        self._add_round_key(s, 0)
        for rnd in range(1, self.rounds):
            for i in range(16):
                s[i] = SBOX[s[i]]
            # ShiftRows（列主序：s[r + 4c]）
            s = [s[r + 4 * ((c + r) % 4)] for c in range(4) for r in range(4)]
            # MixColumns
            s = self._mix_columns(s)
            self._add_round_key(s, rnd)
        for i in range(16):
            s[i] = SBOX[s[i]]
        s = [s[(r + 4 * ((c + r) % 4))] for c in range(4) for r in range(4)]
        self._add_round_key(s, self.rounds)
        return bytes(s)

    @staticmethod
    def _mix_columns(s):
        out = [0] * 16
        for c in range(4):
            a0, a1, a2, a3 = s[4 * c], s[4 * c + 1], s[4 * c + 2], s[4 * c + 3]
            out[4 * c] = MUL2[a0] ^ MUL3[a1] ^ a2 ^ a3
            out[4 * c + 1] = a0 ^ MUL2[a1] ^ MUL3[a2] ^ a3
            out[4 * c + 2] = a0 ^ a1 ^ MUL2[a2] ^ MUL3[a3]
            out[4 * c + 3] = MUL3[a0] ^ a1 ^ a2 ^ MUL2[a3]
        return out

    @staticmethod
    def _inv_mix_columns(s):
        out = [0] * 16
        for c in range(4):
            a0, a1, a2, a3 = s[4 * c], s[4 * c + 1], s[4 * c + 2], s[4 * c + 3]
            out[4 * c] = MUL14[a0] ^ MUL11[a1] ^ MUL13[a2] ^ MUL9[a3]
            out[4 * c + 1] = MUL9[a0] ^ MUL14[a1] ^ MUL11[a2] ^ MUL13[a3]
            out[4 * c + 2] = MUL13[a0] ^ MUL9[a1] ^ MUL14[a2] ^ MUL11[a3]
            out[4 * c + 3] = MUL11[a0] ^ MUL13[a1] ^ MUL9[a2] ^ MUL14[a3]
        return out

    def decrypt_block(self, block: bytes) -> bytes:
        s = list(block)
        self._add_round_key(s, self.rounds)
        for rnd in range(self.rounds - 1, 0, -1):
            s = [s[r + 4 * ((c - r) % 4)] for c in range(4) for r in range(4)]
            for i in range(16):
                s[i] = INV_SBOX[s[i]]
            self._add_round_key(s, rnd)
            s = self._inv_mix_columns(s)
        s = [s[r + 4 * ((c - r) % 4)] for c in range(4) for r in range(4)]
        for i in range(16):
            s[i] = INV_SBOX[s[i]]
        self._add_round_key(s, 0)
        return bytes(s)


def cbc_encrypt(key: bytes, iv: bytes, data: bytes) -> bytes:
    aes = AES256(key)
    out = bytearray()
    prev = iv
    for i in range(0, len(data), 16):
        block = bytes(x ^ y for x, y in zip(data[i:i + 16], prev))
        prev = aes.encrypt_block(block)
        out += prev
    return bytes(out)


_AES_CACHE: dict[bytes, AES256] = {}


def _cbc_decrypt_pure(key: bytes, iv: bytes, data: bytes) -> bytes:
    aes = _AES_CACHE.get(key)
    if aes is None:
        aes = AES256(key)
        _AES_CACHE[key] = aes
    out = bytearray()
    prev = iv
    for i in range(0, len(data), 16):
        block = data[i:i + 16]
        plain = aes.decrypt_block(block)
        out += bytes(x ^ y for x, y in zip(plain, prev))
        prev = block
    return bytes(out)


def _load_native():
    """macOS 上用 CommonCrypto（系统自带、AES-NI 加速）；其他平台返回 None。"""
    import sys

    if sys.platform != "darwin":
        return None
    try:
        import ctypes
        import ctypes.util

        lib = ctypes.CDLL(ctypes.util.find_library("System") or
                          "/usr/lib/libSystem.B.dylib")
        fn = lib.CCCrypt
        fn.restype = ctypes.c_int32
        fn.argtypes = [
            ctypes.c_uint32, ctypes.c_uint32, ctypes.c_uint32,
            ctypes.c_char_p, ctypes.c_size_t, ctypes.c_char_p,
            ctypes.c_char_p, ctypes.c_size_t,
            ctypes.c_char_p, ctypes.c_size_t,
            ctypes.POINTER(ctypes.c_size_t),
        ]
        return fn
    except Exception:
        return None


_NATIVE = _load_native()
_KC_DECRYPT = 1
_KC_ALGORITHM_AES = 0


def cbc_decrypt(key: bytes, iv: bytes, data: bytes) -> bytes:
    if _NATIVE is not None and len(data) % 16 == 0:
        try:
            import ctypes

            out = ctypes.create_string_buffer(len(data))
            moved = ctypes.c_size_t(0)
            status = _NATIVE(_KC_DECRYPT, _KC_ALGORITHM_AES, 0,
                             key, len(key), iv,
                             data, len(data),
                             out, len(data), ctypes.byref(moved))
            if status == 0 and moved.value == len(data):
                return out.raw
        except Exception:
            pass
    return _cbc_decrypt_pure(key, iv, data)
