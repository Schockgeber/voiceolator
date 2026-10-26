#!/usr/bin/env python3
"""Download the runtime files Voiceolator needs next to its executable, with checksums.

    python tools/fetch_deps.py --out dist/voiceolator          # Windows files (default)
    python tools/fetch_deps.py --out . --linux-ort              # also libonnxruntime.so for vo-eval on Linux

Fetches:
  models/dpdfnet2_48khz_hr.onnx  DPDFNet (Apache-2.0), pinned Hugging Face revision
  onnxruntime.dll                ONNX Runtime 1.31.0 (MIT), extracted from the official PyPI wheel
  libonnxruntime.so (--linux-ort) the same for Linux x86_64 (manylinux_2_28 wheel)
Nothing is downloaded at application runtime.
"""
import argparse
import hashlib
import io
import os
import sys
import urllib.request
import zipfile

DPDFNET_REV = "dd6818d00f50c836fed43a6243ebe49116de5964"
DPDFNET_URL = f"https://huggingface.co/Ceva-IP/DPDFNet/resolve/{DPDFNET_REV}/onnx/dpdfnet2_48khz_hr.onnx"
DPDFNET_SHA = "7f0575a5cec0ba4ffd8f8bd657e06d007e4ccdd955d76faab922b9d3291dc14b"
ORT_WHEEL = "onnxruntime-1.31.0-cp312-cp312-win_amd64.whl"
ORT_WHEEL_SHA = "09d56445c1753e66e0912de69d3f0184016ad9a191dcd6925bf5dd570d2bfbe5"
ORT_DLL_SHA = "aa7737cc5c987286f09f6f12304fa786ea697b11b4ca7ab59a027ff4ce343a9c"
ORT_LINUX_WHEEL = "onnxruntime-1.31.0-cp312-cp312-manylinux_2_28_x86_64.whl"
ORT_LINUX_WHEEL_SHA = "5e129d6c56abd53e659cb70f00a108d6824086470ff99c2e47a82e5786563db3"
ORT_SO_SHA = "21901b1b02233484b80888c16f4f5473742f796bf5528d5d0c4073dbe9802001"


def get(url):
    print("downloading", url, file=sys.stderr)
    with urllib.request.urlopen(url) as r:
        return r.read()


def sha(b):
    return hashlib.sha256(b).hexdigest()


def pypi_wheel_url(name, filename):
    import json
    meta = json.loads(get(f"https://pypi.org/pypi/{name}/json"))
    for rel in meta["releases"].values():
        for f in rel:
            if f["filename"] == filename:
                return f["url"]
    raise SystemExit(f"{filename} not found on PyPI")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--linux-ort", action="store_true", help="also fetch libonnxruntime.so (Linux x86_64)")
    a = ap.parse_args()
    os.makedirs(os.path.join(a.out, "models"), exist_ok=True)

    model = get(DPDFNET_URL)
    if sha(model) != DPDFNET_SHA:
        raise SystemExit("DPDFNet checksum mismatch")
    open(os.path.join(a.out, "models", "dpdfnet2_48khz_hr.onnx"), "wb").write(model)

    whl = get(pypi_wheel_url("onnxruntime", ORT_WHEEL))
    if sha(whl) != ORT_WHEEL_SHA:
        raise SystemExit("onnxruntime wheel checksum mismatch")
    with zipfile.ZipFile(io.BytesIO(whl)) as z:
        dll = z.read("onnxruntime/capi/onnxruntime.dll")
        lic = z.read("onnxruntime/LICENSE")
    if sha(dll) != ORT_DLL_SHA:
        raise SystemExit("onnxruntime.dll checksum mismatch")
    open(os.path.join(a.out, "onnxruntime.dll"), "wb").write(dll)
    open(os.path.join(a.out, "LICENSE-onnxruntime.txt"), "wb").write(lic)

    if a.linux_ort:
        whl = get(pypi_wheel_url("onnxruntime", ORT_LINUX_WHEEL))
        if sha(whl) != ORT_LINUX_WHEEL_SHA:
            raise SystemExit("onnxruntime (Linux) wheel checksum mismatch")
        with zipfile.ZipFile(io.BytesIO(whl)) as z:
            so_name = next(n for n in z.namelist() if n.startswith("onnxruntime/capi/libonnxruntime.so"))
            so = z.read(so_name)
        if sha(so) != ORT_SO_SHA:
            raise SystemExit("libonnxruntime.so checksum mismatch")
        open(os.path.join(a.out, "libonnxruntime.so"), "wb").write(so)
    print("ok")


if __name__ == "__main__":
    main()
