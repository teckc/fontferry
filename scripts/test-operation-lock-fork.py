#!/usr/bin/env python3
"""Linux flock mechanism regression; all files are inside TemporaryDirectory.

This demonstrates the fork/open-file-description failure and explicit-unlock
remedy. It complements, not replaces, the Rust OperationGuard tests. It does not
claim to call the Rust guard across a fork or test Windows/macOS lock internals.
"""

import fcntl
import os
import tempfile
from pathlib import Path


def run_case(explicit_unlock: bool) -> None:
    with tempfile.TemporaryDirectory(prefix="fontferry-lock-fork-") as root:
        path = Path(root) / "operation.lock"
        owner = open(path, "a+b")
        fcntl.flock(owner, fcntl.LOCK_EX)
        read_end, write_end = os.pipe()
        pid = os.fork()
        if pid == 0:
            os.close(write_end)
            # Retain the inherited owner descriptor until the parent completes
            # its contention test. No sleeps or scheduling guesses are needed.
            os.read(read_end, 1)
            os.close(read_end)
            owner.close()
            os._exit(0)
        os.close(read_end)
        try:
            if explicit_unlock:
                fcntl.flock(owner, fcntl.LOCK_UN)
            owner.close()
            with open(path, "a+b") as contender:
                acquired = False
                try:
                    fcntl.flock(contender, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    acquired = True
                except BlockingIOError:
                    pass
                assert acquired == explicit_unlock, (
                    f"explicit_unlock={explicit_unlock}, acquired={acquired}"
                )
                if acquired:
                    fcntl.flock(contender, fcntl.LOCK_UN)
        finally:
            os.close(write_end)  # EOF releases the child even after an assertion.
            _, status = os.waitpid(pid, 0)
            assert os.waitstatus_to_exitcode(status) == 0
        with open(path, "a+b") as after_exit:
            fcntl.flock(after_exit, fcntl.LOCK_EX | fcntl.LOCK_NB)
            fcntl.flock(after_exit, fcntl.LOCK_UN)


if __name__ == "__main__":
    if not hasattr(os, "fork"):
        raise SystemExit("This mechanism test requires Unix fork/flock")
    run_case(False)
    run_case(True)
    print("PASS: close-only retains inherited flock; explicit unlock releases it")
