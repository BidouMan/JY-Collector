"""Windows 侧：从正在运行的微信进程内存读取 WCDB 缓存的数据库密钥。

原理（与平台无关的结论已由 4.x 实测确认）：
- 微信 4.x 使用 SQLCipher 4，每个数据库有独立的 salt 与 enc_key；
- WCDB 会把派生后的原始密钥缓存在进程内存里，形态为
  ``x'<64 位十六进制 enc_key><32 位十六进制 salt>'``；
- 扫描进程可读内存匹配该形态，再用目标库首页的 HMAC 校验即可确认。

安全约束：
- 只读进程内存，不注入、不修改微信；
- 密钥只在本进程内使用，不打印、不落盘、不写入日志。

读取进程内存通常需要与微信相同的用户身份；部分环境需要管理员权限，
失败时会返回明确的中文提示，由上层引导用户以管理员身份运行。
"""

from __future__ import annotations

import ctypes
import sys

# 进程/内存相关常量
PROCESS_VM_READ = 0x0010
PROCESS_QUERY_INFORMATION = 0x0400
MEM_COMMIT = 0x1000
PAGE_GUARD = 0x100
READABLE_PROTECT = {
    0x02,  # PAGE_READONLY
    0x04,  # PAGE_READWRITE
    0x08,  # PAGE_WRITECOPY
    0x20,  # PAGE_EXECUTE_READ
    0x40,  # PAGE_EXECUTE_READWRITE
    0x80,  # PAGE_EXECUTE_WRITECOPY
}
TH32CS_SNAPPROCESS = 0x0002
INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value
READ_CHUNK = 8 * 1024 * 1024
OVERLAP = 128

WECHAT_PROCESS_NAMES = ("weixin.exe", "wechat.exe")


def _hex_only(text: bytes) -> bool:
    for byte in text:
        if not (48 <= byte <= 57 or 65 <= byte <= 70 or 97 <= byte <= 102):
            return False
    return True


def _scan_buffer(buffer: bytes) -> list[tuple[str, str]]:
    """在一段内存里匹配 x'<64hex enc_key><32hex salt>'。"""
    found: list[tuple[str, str]] = []
    index = 0
    while True:
        index = buffer.find(b"x'", index)
        if index < 0:
            break
        end = index + 2 + 96 + 1
        if end <= len(buffer):
            segment = buffer[index + 2:index + 98]
            if buffer[index + 98:index + 99] == b"'" and _hex_only(segment):
                text = segment.decode("ascii")
                found.append((text[64:].lower(), text[:64].lower()))
        index += 1
    return found


if sys.platform != "win32":  # pragma: no cover - 仅 Windows 使用
    def scan_key_pairs() -> list[tuple[str, str]]:
        raise RuntimeError("当前平台不支持从微信进程读取密钥")


else:
    from ctypes import wintypes

    _kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

    class _MemoryBasicInformation(ctypes.Structure):
        _fields_ = [
            ("BaseAddress", wintypes.LPVOID),
            ("AllocationBase", wintypes.LPVOID),
            ("AllocationProtect", wintypes.DWORD),
            ("RegionSize", ctypes.c_size_t),
            ("State", wintypes.DWORD),
            ("Protect", wintypes.DWORD),
            ("Type", wintypes.DWORD),
        ]

    class _ProcessEntry32W(ctypes.Structure):
        _fields_ = [
            ("dwSize", wintypes.DWORD),
            ("cntUsage", wintypes.DWORD),
            ("th32ProcessID", wintypes.DWORD),
            ("th32DefaultHeapID", ctypes.POINTER(ctypes.c_ulong)),
            ("th32ModuleID", wintypes.DWORD),
            ("cntThreads", wintypes.DWORD),
            ("th32ParentProcessID", wintypes.DWORD),
            ("pcPriClassBase", ctypes.c_long),
            ("dwFlags", wintypes.DWORD),
            ("szExeFile", wintypes.WCHAR * 260),
        ]

    _kernel32.OpenProcess.restype = wintypes.HANDLE
    _kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    _kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    _kernel32.VirtualQueryEx.restype = ctypes.c_size_t
    _kernel32.VirtualQueryEx.argtypes = [
        wintypes.HANDLE,
        wintypes.LPCVOID,
        ctypes.POINTER(_MemoryBasicInformation),
        ctypes.c_size_t,
    ]
    _kernel32.ReadProcessMemory.restype = wintypes.BOOL
    _kernel32.ReadProcessMemory.argtypes = [
        wintypes.HANDLE,
        wintypes.LPCVOID,
        wintypes.LPVOID,
        ctypes.c_size_t,
        ctypes.POINTER(ctypes.c_size_t),
    ]
    _kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    _kernel32.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
    _kernel32.Process32FirstW.restype = wintypes.BOOL
    _kernel32.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(_ProcessEntry32W)]
    _kernel32.Process32NextW.restype = wintypes.BOOL
    _kernel32.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(_ProcessEntry32W)]

    def _process_ids() -> list[int]:
        snapshot = _kernel32.CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
        if not snapshot or snapshot == INVALID_HANDLE_VALUE:
            return []
        entries = _ProcessEntry32W()
        entries.dwSize = ctypes.sizeof(_ProcessEntry32W)
        ids: list[int] = []
        try:
            ok = _kernel32.Process32FirstW(snapshot, ctypes.byref(entries))
            while ok:
                name = entries.szExeFile.lower()
                if name in WECHAT_PROCESS_NAMES:
                    ids.append(int(entries.th32ProcessID))
                ok = _kernel32.Process32NextW(snapshot, ctypes.byref(entries))
        finally:
            _kernel32.CloseHandle(snapshot)
        return ids

    def _scan_process(pid: int) -> tuple[list[tuple[str, str]], bool]:
        """返回 (密钥对, 是否因权限被拒绝)。"""
        handle = _kernel32.OpenProcess(
            PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, False, pid)
        if not handle:
            error = ctypes.get_last_error()
            return [], error == 5
        found: list[tuple[str, str]] = []
        address = 0
        try:
            while True:
                info = _MemoryBasicInformation()
                size = _kernel32.VirtualQueryEx(
                    handle, ctypes.c_void_p(address), ctypes.byref(info),
                    ctypes.sizeof(info))
                if not size:
                    break
                base = int(info.BaseAddress or 0)
                region_size = int(info.RegionSize)
                if base == 0 and region_size == 0:
                    break
                readable = (
                    info.State == MEM_COMMIT
                    and not (info.Protect & PAGE_GUARD)
                    and (info.Protect & 0xFF) in READABLE_PROTECT
                )
                if readable and 0 < region_size <= 512 * 1024 * 1024:
                    offset = 0
                    overlap = b""
                    while offset < region_size:
                        length = min(READ_CHUNK, region_size - offset)
                        buffer = ctypes.create_string_buffer(length)
                        read = ctypes.c_size_t(0)
                        ok = _kernel32.ReadProcessMemory(
                            handle, ctypes.c_void_p(base + offset), buffer,
                            length, ctypes.byref(read))
                        if ok and read.value:
                            chunk = buffer.raw[:read.value]
                            found.extend(_scan_buffer(overlap + chunk))
                            overlap = chunk[-OVERLAP:]
                        else:
                            overlap = b""
                        offset += length
                address = base + region_size
        finally:
            _kernel32.CloseHandle(handle)
        return found, False

    def scan_key_pairs() -> list[tuple[str, str]]:
        """扫描所有微信进程，返回去重后的 (salt_hex, key_hex) 列表。"""
        pids = _process_ids()
        if not pids:
            raise RuntimeError(
                "没有找到正在运行的微信（Weixin.exe），请先登录微信后再检测")
        pairs: dict[tuple[str, str], None] = {}
        denied = False
        for pid in pids:
            try:
                found, access_denied = _scan_process(pid)
            except Exception:  # noqa: BLE001 - 单个进程失败不影响其他进程
                continue
            denied = denied or access_denied
            for pair in found:
                pairs[pair] = None
        if pairs:
            return list(pairs.keys())
        if denied:
            raise RuntimeError(
                "无法读取微信进程内存：请右键应用选择“以管理员身份运行”后重试")
        raise RuntimeError(
            "微信进程已找到，但未读取到数据库密钥；请确认微信已登录并打开过聊天窗口后重试")
