"""Cold-start latency: mean and p90 wall time of N runs per command."""
import os
import shutil
import statistics
import subprocess
import time

S = "/Users/joel.chen/dev/safesh-rs/.temp/brush-spike"
P = f"{S}/policy-target/release/brush-policy-spike"
N = 50
os.chdir(f"{S}/work")

cases = [
    ("bash -c true", ["/bin/bash", "-c", "true"]),
    ("brush-stock -c true", [f"{S}/bin/brush-stock", "--norc", "--noprofile", "-c", "true"]),
    ("brush-policy -c true", [P, "-c", "true"]),
    ("bash pipeline", ["/bin/bash", "-c", "ls | grep f | wc -l"]),
    ("brush-policy pipeline", [P, "-c", "ls | grep f | wc -l"]),
    ("brush-policy nested bash -c", [P, "-c", "bash -c true"]),
]
deno = shutil.which("deno")
if deno:
    cases.append(("deno eval 0 (desh floor)", [deno, "eval", "0"]))

for label, cmd in cases:
    times = []
    for _ in range(N):
        t0 = time.perf_counter()
        subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
        times.append((time.perf_counter() - t0) * 1000)
    times.sort()
    print(f"{label:30s} mean {statistics.mean(times):6.1f} ms   p90 {times[int(N * 0.9)]:6.1f} ms")

print(f"binary size: {os.path.getsize(P) / 1048576:.1f} MB")
