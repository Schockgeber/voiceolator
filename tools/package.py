#!/usr/bin/env python3
"""Build all release files for Windows and Linux.

    python tools/package.py                     # Windows + Linux, build everything
    python tools/package.py --only windows      # or: --only linux
    python tools/package.py --skip-build        # reuse existing binaries
    python tools/package.py --bundle            # also write the single GitHub-ready zip

Produces in dist/:
    Voiceolator-<ver>-windows-setup.exe          per-user installer (NSIS)
    Voiceolator-<ver>-windows-portable.zip       unzip and run
    Voiceolator-<ver>-windows-cli.zip            command-line tool
    Voiceolator-<ver>-linux-x86_64.AppImage      download, chmod +x, run
    Voiceolator-<ver>-linux-x86_64.tar.gz        portable (+ install.sh for a menu entry)
    Voiceolator-<ver>-linux-cli-x86_64.tar.gz    command-line tool
    SHA256SUMS.txt
and with --bundle: Voiceolator-<ver>-github-ready.zip (repository + all of the above).

Build hosts:
    * Linux (used for releases): mingw-w64 + rustup target x86_64-pc-windows-gnu for
      Windows; zig + cargo-zigbuild + libpulse-dev for Linux (targets glibc 2.31, i.e.
      Ubuntu 20.04 / Debian 11 and newer); nsis (makensis); appimagetool on PATH or
      APPIMAGETOOL=...; Python package `markdown`.
    * Windows/MSVC: --only windows --windows-target x86_64-pc-windows-msvc (needs NSIS).
"""
import argparse
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tarfile
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DIST = os.path.join(ROOT, "dist")
LINUX_TARGET = "x86_64-unknown-linux-gnu"
LINUX_GLIBC = "2.31"


def version():
    txt = open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8").read()
    return re.search(r'^version\s*=\s*"([^"]+)"', txt, re.M).group(1)


HTML = """<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>{title}</title>
<style>
:root{{color-scheme:dark}} body{{background:#12141c;color:#dadeea;font:15px/1.55 "Segoe UI",system-ui,sans-serif;max-width:860px;margin:0 auto;padding:24px 18px 60px}}
h1{{color:#fff;font-size:28px}} h2{{color:#2ed6c4;margin-top:34px;border-bottom:1px solid #2a2e3e;padding-bottom:6px}}
a{{color:#2ed6c4}} code{{background:#1c1f2b;padding:1px 5px;border-radius:4px}}
table{{border-collapse:collapse;width:100%;margin:12px 0}} th,td{{border:1px solid #2a2e3e;padding:7px 9px;text-align:left;vertical-align:top}}
th{{background:#1c1f2b}} tr:nth-child(even) td{{background:#161923}} li{{margin:4px 0}}
</style></head><body>{body}</body></html>"""


def md_to_html(md_path, out_path, title):
    import markdown
    body = markdown.markdown(open(md_path, encoding="utf-8").read(), extensions=["tables", "fenced_code"])
    open(out_path, "w", encoding="utf-8").write(HTML.format(title=title, body=body))


def sha256(p):
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def run(cmd, **kw):
    print("+", " ".join(cmd), flush=True)
    subprocess.run(cmd, check=True, **kw)


def deps(cache, linux):
    need = os.path.join(cache, "libonnxruntime.so" if linux else "onnxruntime.dll")
    if not os.path.exists(need):
        args = [sys.executable, os.path.join(ROOT, "tools", "fetch_deps.py"), "--out", cache]
        if linux:
            args.append("--linux-ort")
        run(args)


def common_docs(stage, newline="\n"):
    md_to_html(os.path.join(ROOT, "docs", "USER_GUIDE.md"), os.path.join(stage, "README.html"), "Voiceolator user guide")
    lic = open(os.path.join(ROOT, "LICENSE-MIT"), encoding="utf-8").read()
    txt = ("Voiceolator is dual-licensed under the MIT license or the Apache License 2.0, at your option.\n"
           "The MIT license text follows; the Apache-2.0 text is in the source repository (LICENSE-APACHE).\n\n" + lic)
    open(os.path.join(stage, "LICENSE.txt"), "w", encoding="utf-8", newline=newline).write(txt)
    shutil.copy(os.path.join(ROOT, "THIRD_PARTY_LICENSES.md"), os.path.join(stage, "THIRD_PARTY_LICENSES.txt"))


def zipdir(src, out, prefix):
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for base, _, files in os.walk(src):
            for f in sorted(files):
                p = os.path.join(base, f)
                z.write(p, os.path.join(prefix, os.path.relpath(p, src)))


def tardir(src, out, prefix):
    with tarfile.open(out, "w:gz", compresslevel=9) as t:
        t.add(src, arcname=prefix)


# --------------------------------------------------------------------------- Windows

def package_windows(ver, a):
    target = a.windows_target
    if not a.skip_build:
        run(["cargo", "build", "--release", "--target", target, "-p", "vo-gui", "-p", "vo-cli"], cwd=ROOT)
    bindir = os.path.join(ROOT, "target", target, "release")
    cache = os.path.join(ROOT, "target", "deps-cache")
    deps(cache, linux=False)
    stage = os.path.join(DIST, "stage-windows", f"Voiceolator-{ver}")
    shutil.rmtree(os.path.dirname(stage), ignore_errors=True)
    os.makedirs(os.path.join(stage, "models"))
    shutil.copy(os.path.join(bindir, "Voiceolator.exe"), stage)
    shutil.copy(os.path.join(cache, "onnxruntime.dll"), stage)
    shutil.copy(os.path.join(cache, "LICENSE-onnxruntime.txt"), stage)
    shutil.copy(os.path.join(cache, "models", "dpdfnet2_48khz_hr.onnx"), os.path.join(stage, "models"))
    shutil.copy(os.path.join(ROOT, "crates", "vo-gui", "assets", "icon.ico"), stage)
    common_docs(stage, newline="\r\n")

    out = []
    portable = os.path.join(DIST, f"Voiceolator-{ver}-windows-portable.zip")
    zipdir(stage, portable, f"Voiceolator-{ver}")
    out.append(portable)

    cli = os.path.join(DIST, f"Voiceolator-{ver}-windows-cli.zip")
    with zipfile.ZipFile(cli, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        z.write(os.path.join(bindir, "voiceolator-cli.exe"), "voiceolator-cli.exe")
        z.writestr("README.txt", "Copy voiceolator-cli.exe into your Voiceolator folder (next to onnxruntime.dll and models\\).\r\n"
                   "Run `voiceolator-cli help` in a terminal. See docs/DEVELOPMENT.md for the commands.\r\n")
    out.append(cli)

    setup = os.path.join(DIST, f"Voiceolator-{ver}-windows-setup.exe")
    run(["makensis", "-V2", f"-DVERSION={ver}", f"-DSRCDIR={stage}", f"-DOUTFILE={setup}", os.path.join(ROOT, "installer", "voiceolator.nsi")])
    out.insert(0, setup)
    return out


# --------------------------------------------------------------------------- Linux

def package_linux(ver, a):
    target = f"{LINUX_TARGET}.{LINUX_GLIBC}"
    if not a.skip_build:
        env = dict(os.environ)
        link = os.path.join(ROOT, "target", "pulse-link")
        os.makedirs(link, exist_ok=True)
        for lib in ("libpulse.so", "libpulse-simple.so"):
            dst = os.path.join(link, lib)
            if not os.path.exists(dst):
                for d in ("/usr/lib/x86_64-linux-gnu", "/usr/lib64", "/usr/lib"):
                    if os.path.exists(os.path.join(d, lib + ".0")):
                        os.symlink(os.path.join(d, lib + ".0"), dst)
                        break
        env["RUSTFLAGS"] = (env.get("RUSTFLAGS", "") + f" -L {link}").strip()
        run(["cargo", "zigbuild", "--release", "--target", target, "-p", "vo-gui", "-p", "vo-cli"], cwd=ROOT, env=env)
    bindir = os.path.join(ROOT, "target", LINUX_TARGET, "release")
    cache = os.path.join(ROOT, "target", "deps-cache-linux")
    deps(cache, linux=True)
    pk = os.path.join(ROOT, "packaging", "linux")

    def stage_app(dest):
        os.makedirs(os.path.join(dest, "models"), exist_ok=True)
        shutil.copy(os.path.join(bindir, "Voiceolator"), os.path.join(dest, "voiceolator"))
        shutil.copy(os.path.join(cache, "libonnxruntime.so"), dest)
        shutil.copy(os.path.join(cache, "LICENSE-onnxruntime.txt"), dest)
        shutil.copy(os.path.join(cache, "models", "dpdfnet2_48khz_hr.onnx"), os.path.join(dest, "models"))
        common_docs(dest)
        os.chmod(os.path.join(dest, "voiceolator"), 0o755)

    out = []
    # AppImage
    tool = a.appimagetool or shutil.which("appimagetool")
    if tool:
        appdir = os.path.join(DIST, "stage-linux", "Voiceolator.AppDir")
        shutil.rmtree(appdir, ignore_errors=True)
        stage_app(os.path.join(appdir, "usr", "lib", "voiceolator"))
        shutil.copy(os.path.join(pk, "AppRun"), appdir)
        os.chmod(os.path.join(appdir, "AppRun"), 0o755)
        shutil.copy(os.path.join(pk, "voiceolator.desktop"), appdir)
        shutil.copy(os.path.join(ROOT, "crates", "vo-gui", "assets", "icon.png"), os.path.join(appdir, "voiceolator.png"))
        appimage = os.path.join(DIST, f"Voiceolator-{ver}-linux-x86_64.AppImage")
        env = dict(os.environ, ARCH="x86_64", VERSION=ver)
        cmd = [tool]
        if a.appimage_extract_and_run:
            cmd.append("--appimage-extract-and-run")
        run(cmd + ["--no-appstream", appdir, appimage], env=env)
        out.append(appimage)
    else:
        print("appimagetool not found: skipping the AppImage", file=sys.stderr)

    # portable tarball with install.sh
    stage = os.path.join(DIST, "stage-linux", f"Voiceolator-{ver}")
    shutil.rmtree(stage, ignore_errors=True)
    stage_app(stage)
    for f in ("install.sh", "voiceolator.desktop"):
        shutil.copy(os.path.join(pk, f), stage)
    os.chmod(os.path.join(stage, "install.sh"), 0o755)
    shutil.copy(os.path.join(ROOT, "crates", "vo-gui", "assets", "icon.png"), os.path.join(stage, "voiceolator.png"))
    tgz = os.path.join(DIST, f"Voiceolator-{ver}-linux-x86_64.tar.gz")
    tardir(stage, tgz, f"Voiceolator-{ver}")
    out.append(tgz)

    cli_stage = os.path.join(DIST, "stage-linux", f"voiceolator-cli-{ver}")
    shutil.rmtree(cli_stage, ignore_errors=True)
    os.makedirs(cli_stage)
    shutil.copy(os.path.join(bindir, "voiceolator-cli"), cli_stage)
    os.chmod(os.path.join(cli_stage, "voiceolator-cli"), 0o755)
    open(os.path.join(cli_stage, "README.txt"), "w").write(
        "Copy voiceolator-cli into your Voiceolator folder (next to libonnxruntime.so and models/),\n"
        "or into ~/.local/share/voiceolator after running install.sh. Run `voiceolator-cli help`.\n")
    cli = os.path.join(DIST, f"Voiceolator-{ver}-linux-cli-x86_64.tar.gz")
    tardir(cli_stage, cli, f"voiceolator-cli-{ver}")
    out.append(cli)
    return out


# --------------------------------------------------------------------------- bundle

PUBLISH = """# Publishing Voiceolator {ver} on GitHub

This zip contains everything:

* `voiceolator/`: the source repository. Push this folder to GitHub.
* `release-assets/`: the ready-built downloads. Attach these to a GitHub Release.

## 1. Push the source

```
cd voiceolator
git init
git add .
git commit -m "Voiceolator {ver}"
git branch -M main
git remote add origin https://github.com/<your-account>/voiceolator.git
git push -u origin main
```

`.github/workflows/ci.yml` builds and tests Windows and Linux on every push.

## 2. Create the release

On GitHub: **Releases → Draft a new release**, tag `v{ver}`, title `Voiceolator {ver}`.
Drag in every file from `release-assets/`, paste the matching part of `CHANGELOG.md` as
the description, and publish.

Pushing a tag (`git tag v{ver} && git push --tags`) makes the CI workflow build the same
files on GitHub and attach them to the release automatically. Use either the files from
this zip or the CI build, not both.

## Downloads for users

| File | For |
|---|---|
{table}

`SHA256SUMS.txt` lets people check their download.
"""

DESCR = {
    "windows-setup.exe": "Windows 10 (2004+) / 11: installer, the recommended choice",
    "windows-portable.zip": "Windows: unzip and run `Voiceolator.exe`",
    "windows-cli.zip": "Windows: command-line tool (power users)",
    "linux-x86_64.AppImage": "Linux: download, `chmod +x`, run",
    "linux-x86_64.tar.gz": "Linux: portable; `./install.sh` adds a menu entry",
    "linux-cli-x86_64.tar.gz": "Linux: command-line tool (power users)",
}


def bundle(ver, files):
    out = os.path.join(DIST, f"Voiceolator-{ver}-github-ready.zip")
    src_zip = os.path.join(DIST, f"Voiceolator-{ver}-source.zip")
    run(["git", "archive", "--format=zip", f"--prefix=voiceolator/", "-o", src_zip, "HEAD"], cwd=ROOT)
    rows = []
    for f in files:
        name = os.path.basename(f)
        key = name.split(f"{ver}-", 1)[-1]
        rows.append(f"| `{name}` | {DESCR.get(key, '')} |")
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as z:
        with zipfile.ZipFile(src_zip) as s:
            for info in s.infolist():
                z.writestr(info, s.read(info.filename))
        for f in files + [os.path.join(DIST, "SHA256SUMS.txt")]:
            info = zipfile.ZipInfo.from_file(f, os.path.join("release-assets", os.path.basename(f)))
            info.compress_type = zipfile.ZIP_STORED if f.endswith((".zip", ".gz", ".exe", ".AppImage")) else zipfile.ZIP_DEFLATED
            if f.endswith(".AppImage"):
                info.external_attr = (0o755 << 16)
            with open(f, "rb") as fh:
                z.writestr(info, fh.read())
        z.writestr("HOW-TO-PUBLISH.md", PUBLISH.format(ver=ver, table="\n".join(rows)))
    os.remove(src_zip)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", choices=["windows", "linux"])
    ap.add_argument("--skip-build", action="store_true")
    ap.add_argument("--windows-target", default="x86_64-pc-windows-gnu")
    ap.add_argument("--appimagetool", default=os.environ.get("APPIMAGETOOL"))
    ap.add_argument("--appimage-extract-and-run", action="store_true", help="for build hosts without FUSE")
    ap.add_argument("--bundle", action="store_true")
    a = ap.parse_args()
    ver = version()
    os.makedirs(DIST, exist_ok=True)
    files = []
    if a.only in (None, "windows"):
        files += package_windows(ver, a)
    if a.only in (None, "linux"):
        files += package_linux(ver, a)
    with open(os.path.join(DIST, "SHA256SUMS.txt"), "w") as f:
        for p in files:
            f.write(f"{sha256(p)}  {os.path.basename(p)}\n")
    for p in files:
        print(f"{os.path.getsize(p) / 1e6:6.1f} MB  {p}")
    if a.bundle:
        b = bundle(ver, files)
        print(f"{os.path.getsize(b) / 1e6:6.1f} MB  {b}")


if __name__ == "__main__":
    main()
