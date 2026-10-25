# SPDX-License-Identifier: Apache-2.0
"""Windows child resource sampling. Imported safely on non-Windows hosts.

CPU times are final process totals. Accepted live peak-working-set samples are
only a lower bound on lifetime peak; no post-exit memory guarantee is assumed.
"""

import ctypes
from ctypes import wintypes
import subprocess
import time


class WindowsResources:
    def __init__(self, pid):
        self.errors = []
        self.peak = None
        self.samples = 0
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)

        class Memory(ctypes.Structure):
            _fields_ = [
                ("cb", wintypes.DWORD),
                ("PageFaultCount", wintypes.DWORD),
            ] + [
                (name, ctypes.c_size_t) for name in (
                    "PeakWorkingSetSize", "WorkingSetSize",
                    "QuotaPeakPagedPoolUsage", "QuotaPagedPoolUsage",
                    "QuotaPeakNonPagedPoolUsage", "QuotaNonPagedPoolUsage",
                    "PagefileUsage", "PeakPagefileUsage",
                )
            ]

        self.memory_type = Memory
        self.kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        self.kernel.OpenProcess.restype = wintypes.HANDLE
        self.kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        self.kernel.CloseHandle.restype = wintypes.BOOL
        self.kernel.K32GetProcessMemoryInfo.argtypes = [
            wintypes.HANDLE, ctypes.POINTER(Memory), wintypes.DWORD,
        ]
        self.kernel.K32GetProcessMemoryInfo.restype = wintypes.BOOL
        self.kernel.GetProcessTimes.argtypes = [
            wintypes.HANDLE,
        ] + [ctypes.POINTER(wintypes.FILETIME)] * 4
        self.kernel.GetProcessTimes.restype = wintypes.BOOL
        self.handle = self.kernel.OpenProcess(0x1000, False, pid)
        if not self.handle:
            self.error("OpenProcess")

    def error(self, api):
        code = ctypes.get_last_error()
        # Keep only the first occurrence of a repeated API failure.
        value = {"api": api, "winerror": code}
        if value not in self.errors:
            self.errors.append(value)

    def sample(self, process):
        if not self.handle or process.poll() is not None:
            return
        memory = self.memory_type()
        memory.cb = ctypes.sizeof(memory)
        if not self.kernel.K32GetProcessMemoryInfo(
            self.handle, ctypes.byref(memory), memory.cb
        ):
            self.error("K32GetProcessMemoryInfo")
        elif process.poll() is None:
            self.peak = max(self.peak or 0, memory.PeakWorkingSetSize)
            self.samples += 1

    def finish(self):
        user_ns = kernel_ns = None
        if self.handle:
            try:
                created, exited, kernel, user = [wintypes.FILETIME() for _ in range(4)]
                if self.kernel.GetProcessTimes(
                    self.handle, ctypes.byref(created), ctypes.byref(exited),
                    ctypes.byref(kernel), ctypes.byref(user),
                ):
                    user_ns = ((user.dwHighDateTime << 32) | user.dwLowDateTime) * 100
                    kernel_ns = (
                        (kernel.dwHighDateTime << 32) | kernel.dwLowDateTime
                    ) * 100
                else:
                    self.error("GetProcessTimes")
            finally:
                if not self.kernel.CloseHandle(self.handle):
                    self.error("CloseHandle")
                self.handle = None
        return {
            "method": "Windows process query handle; live OS-peak samples",
            "cpu_user_ns": user_ns,
            "cpu_kernel_ns": kernel_ns,
            "cpu_quality": "final" if user_ns is not None else "unavailable",
            "peak_working_set_bytes": self.peak,
            "memory_quality": "lower_bound" if self.samples else "unavailable",
            "memory_samples": self.samples,
            "requested_sample_interval_ms": 10,
            "errors": self.errors,
        }


def capture(command, cwd, env, stdout, stderr, timeout_seconds=1800):
    process = subprocess.Popen(
        command, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
        stdout=stdout, stderr=stderr, shell=False,
    )
    resources = None
    initialization_error = None
    deadline = time.monotonic() + timeout_seconds
    try:
        try:
            resources = WindowsResources(process.pid)
        except (OSError, AttributeError) as error:
            initialization_error = str(error)
        if resources is None:
            process.wait(timeout=timeout_seconds)
        else:
            while process.poll() is None:
                if time.monotonic() >= deadline:
                    raise subprocess.TimeoutExpired(command, timeout_seconds)
                resources.sample(process)
                time.sleep(0.01)
        process.wait()
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait()
        result = resources.finish() if resources is not None else {
            "method": "Windows resource API initialization failed",
            "memory_quality": "unavailable",
            "peak_working_set_bytes": None,
            "cpu_quality": "unavailable",
            "cpu_user_ns": None,
            "cpu_kernel_ns": None,
            "error": initialization_error,
        }
    return process.returncode, result
