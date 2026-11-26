"""Bounded singleton acquisition, including an owner exiting during startup."""
import json
import os
import time
from urllib.request import Request, ProxyHandler, build_opener


class StartupUnavailable(RuntimeError):
    pass


def lock_file(path):
    file = open(path, "a+b")
    try:
        if os.name == "nt":
            import msvcrt
            file.seek(0)
            if not file.read(1):
                file.write(b"0")
                file.flush()
            file.seek(0)
            msvcrt.locking(file.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl
            fcntl.flock(file.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        return file
    except OSError:
        file.close()
        return None


def acquire_service(root, config, timeout=8):
    opener = build_opener(ProxyHandler({}))
    deadline = time.monotonic() + timeout
    error = "service_unavailable"
    while True:
        # The owner can disappear at any point, including before publishing its
        # endpoint. Never stay permanently in the attacher branch after that.
        owner = lock_file(root / "service.lock")
        if owner:
            return owner, None
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise StartupUnavailable(error)
        try:
            info = json.loads((root / "runtime.json").read_text())
            request = Request(info["endpoint"] + "/health", headers={"Authorization":"Bearer " + info["token"]})
            with opener.open(request, timeout=min(0.3, remaining)) as response:
                health = json.load(response)
            if (isinstance(health, dict) and health.get("protocol") == 1 and health.get("fingerprint") == config["fingerprint"]
                    and health.get("bundleId", "") == config.get("bundleId", "")):
                return None, info
            error = "incompatible_service"
        except (OSError, ValueError, KeyError, TypeError):
            pass
        time.sleep(min(0.05, max(0, deadline - time.monotonic())))
