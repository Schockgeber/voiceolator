//! Finding ffmpeg, and downloading it for the user when they agree.
//!
//! Voiceolator never bundles ffmpeg. It looks for an existing copy (next to Voiceolator,
//! in Voiceolator's data folder, on PATH and in a few common install locations). If none
//! is found and the user agrees, [`download`] fetches one pinned, checksum-verified build
//! from its publisher, and extracts only `ffmpeg` (plus its licence) into the data folder:
//!
//! * Windows x64: gyan.dev "essentials" build (GPL-3.0).
//! * Linux x86_64: johnvansickle.com static build (GPL-3.0).

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub struct Ffmpeg {
    pub path: PathBuf,
    pub version: String,
}

pub fn exe_name() -> &'static str {
    if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

/// Where a downloaded ffmpeg lives.
pub fn managed_dir() -> PathBuf {
    crate::data_dir().join("ffmpeg")
}

/// Run `ffmpeg -version`; Some if it works.
pub fn check(path: &Path) -> Option<Ffmpeg> {
    let out = crate::hidden_command(path).args(["-hide_banner", "-version"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let first = s.lines().next()?;
    let version = first.strip_prefix("ffmpeg version ")?.split_whitespace().next()?.to_string();
    Some(Ffmpeg { path: path.to_path_buf(), version })
}

/// Does this ffmpeg have the named encoder (e.g. "libopus", "libmp3lame")?
pub fn has_encoder(path: &Path, name: &str) -> bool {
    crate::hidden_command(path)
        .args(["-hide_banner", "-encoders"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.split_whitespace().nth(1) == Some(name)))
        .unwrap_or(false)
}

fn candidates() -> Vec<PathBuf> {
    let name = exe_name();
    let mut v = Vec::new();
    if let Some(p) = std::env::var_os("VOICEOLATOR_FFMPEG") {
        v.push(PathBuf::from(p));
    }
    if let Some(d) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        v.push(d.join(name));
    }
    v.push(managed_dir().join(name));
    if let Some(path) = std::env::var_os("PATH") {
        v.extend(std::env::split_paths(&path).map(|d| d.join(name)));
    }
    #[cfg(windows)]
    {
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            v.push(PathBuf::from(l).join("Microsoft").join("WinGet").join("Links").join(name));
        }
        for base in ["ProgramFiles", "ProgramData", "ChocolateyInstall"] {
            if let Some(b) = std::env::var_os(base) {
                v.push(PathBuf::from(&b).join("ffmpeg").join("bin").join(name));
                v.push(PathBuf::from(&b).join("bin").join(name));
            }
        }
        if let Some(h) = std::env::var_os("USERPROFILE") {
            v.push(PathBuf::from(h).join("scoop").join("shims").join(name));
        }
        v.push(PathBuf::from(r"C:\ffmpeg\bin").join(name));
    }
    #[cfg(not(windows))]
    {
        for d in ["/usr/bin", "/usr/local/bin", "/snap/bin", "/var/lib/flatpak/exports/bin", "/opt/homebrew/bin"] {
            v.push(PathBuf::from(d).join(name));
        }
        if let Some(h) = std::env::var_os("HOME") {
            v.push(PathBuf::from(h).join(".local").join("bin").join(name));
        }
    }
    v
}

/// Look for a working ffmpeg. Takes a few milliseconds per candidate that exists.
pub fn find() -> Option<Ffmpeg> {
    let mut seen = Vec::new();
    for c in candidates() {
        if !c.is_file() || seen.contains(&c) {
            continue;
        }
        seen.push(c.clone());
        if let Some(f) = check(&c) {
            return Some(f);
        }
    }
    None
}

/// A pinned ffmpeg build for this platform.
pub struct DownloadInfo {
    /// Mirrors of the same file; tried in order.
    pub urls: &'static [&'static str],
    pub sha256: &'static str,
    pub size: u64,
    /// Path of the ffmpeg executable inside the archive.
    pub member: &'static str,
    pub license_member: &'static str,
    pub version: &'static str,
    pub publisher: &'static str,
    pub license: &'static str,
}

#[cfg(windows)]
pub const DOWNLOAD: Option<DownloadInfo> = Some(DownloadInfo {
    urls: &[
        "https://www.gyan.dev/ffmpeg/builds/packages/ffmpeg-9.0.2-essentials_build.zip",
        "https://github.com/GyanD/codexffmpeg/releases/download/9.0.2/ffmpeg-9.0.2-essentials_build.zip",
    ],
    sha256: "60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba",
    size: 114_768_076,
    member: "ffmpeg-9.0.2-essentials_build/bin/ffmpeg.exe",
    license_member: "ffmpeg-9.0.2-essentials_build/LICENSE",
    version: "9.0.2",
    publisher: "gyan.dev",
    license: "GPL-3.0",
});

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const DOWNLOAD: Option<DownloadInfo> = Some(DownloadInfo {
    urls: &[
        "https://johnvansickle.com/ffmpeg/releases/ffmpeg-7.0.2-amd64-static.tar.xz",
        "https://johnvansickle.com/ffmpeg/old-releases/ffmpeg-7.0.2-amd64-static.tar.xz",
    ],
    sha256: "abda8d77ce8309141f83ab8edf0596834087c52467f6badf376a6a2a4c87cf67",
    size: 41_888_096,
    member: "ffmpeg-7.0.2-amd64-static/ffmpeg",
    license_member: "ffmpeg-7.0.2-amd64-static/GPLv3.txt",
    version: "7.0.2",
    publisher: "johnvansickle.com",
    license: "GPL-3.0",
});

#[cfg(not(any(windows, all(target_os = "linux", target_arch = "x86_64"))))]
pub const DOWNLOAD: Option<DownloadInfo> = None;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DownloadStage {
    Downloading { done: u64, total: u64 },
    Extracting,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .try_proxy_from_env(true)
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(60))
        .user_agent(concat!("Voiceolator/", env!("CARGO_PKG_VERSION")))
        .build()
}

fn fetch(url: &str, info: &DownloadInfo, dest: &Path, progress: &mut dyn FnMut(DownloadStage), cancel: &AtomicBool) -> Result<()> {
    let resp = agent().get(url).call().with_context(|| format!("could not reach {url}"))?;
    let mut body = resp.into_reader();
    let mut f = std::fs::File::create(dest).with_context(|| format!("cannot write {}", dest.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    let mut last = std::time::Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!(crate::Cancelled);
        }
        let n = body.read(&mut buf).context("download interrupted")?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        f.write_all(&buf[..n])?;
        done += n as u64;
        if done > info.size + 1024 {
            bail!("download is larger than expected");
        }
        if last.elapsed() > Duration::from_millis(150) {
            last = std::time::Instant::now();
            progress(DownloadStage::Downloading { done, total: info.size });
        }
    }
    f.flush()?;
    let got = format!("{:x}", h.finalize());
    if done != info.size || got != info.sha256 {
        bail!("the downloaded file failed the integrity check (size {done}, sha256 {got})");
    }
    progress(DownloadStage::Downloading { done, total: info.size });
    Ok(())
}

fn write_atomic(dest: &Path, data: &mut dyn Read, executable: bool) -> Result<()> {
    let tmp = dest.with_extension("part");
    {
        let mut f = std::fs::File::create(&tmp).with_context(|| format!("cannot write {}", tmp.display()))?;
        std::io::copy(data, &mut f)?;
        f.flush()?;
    }
    #[cfg(unix)]
    if executable {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = executable;
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

#[cfg(windows)]
fn extract(archive: &Path, info: &DownloadInfo, dir: &Path) -> Result<()> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(archive)?).context("the download is not a valid zip file")?;
    write_atomic(&dir.join(exe_name()), &mut z.by_name(info.member).context("ffmpeg.exe not found in the download")?, true)?;
    if let Ok(mut l) = z.by_name(info.license_member) {
        let _ = write_atomic(&dir.join("LICENSE.txt"), &mut l, false);
    }
    Ok(())
}

#[cfg(not(windows))]
fn extract(archive: &Path, info: &DownloadInfo, dir: &Path) -> Result<()> {
    // xz -> tar through a pipe, so the 170 MB tar never touches the disk.
    let (reader, mut writer) = std::io::pipe()?;
    let src = archive.to_path_buf();
    let unxz = std::thread::spawn(move || -> Result<()> {
        let mut input = std::io::BufReader::new(std::fs::File::open(src)?);
        lzma_rs::xz_decompress(&mut input, &mut writer).map_err(|e| anyhow::anyhow!("xz: {e:?}"))?;
        Ok(())
    });
    let mut found = false;
    let mut t = tar::Archive::new(reader);
    for entry in t.entries()? {
        let mut e = entry?;
        let p = e.path()?.to_string_lossy().to_string();
        if p == info.member {
            write_atomic(&dir.join(exe_name()), &mut e, true)?;
            found = true;
        } else if p == info.license_member {
            let _ = write_atomic(&dir.join("LICENSE.txt"), &mut e, false);
        }
    }
    drop(t);
    // The reader is gone; a "broken pipe" from the decompressor after we got what we
    // need is expected.
    let r = unxz.join().map_err(|_| anyhow::anyhow!("xz thread panicked"))?;
    if !found {
        r?;
        bail!("ffmpeg not found in the download");
    }
    Ok(())
}

/// Download, verify and unpack ffmpeg into [`managed_dir`]. Only call after the user agreed.
pub fn download(progress: &mut dyn FnMut(DownloadStage), cancel: &AtomicBool) -> Result<Ffmpeg> {
    let info = DOWNLOAD.as_ref().context("there is no ffmpeg download for this system; please install ffmpeg yourself")?;
    let dir = managed_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let archive = dir.join("download.tmp");
    let mut errors = Vec::new();
    let mut ok = false;
    for url in info.urls {
        match fetch(url, info, &archive, progress, cancel) {
            Ok(()) => {
                ok = true;
                break;
            }
            Err(e) if e.is::<crate::Cancelled>() => {
                let _ = std::fs::remove_file(&archive);
                return Err(e);
            }
            Err(e) => {
                log::warn!("ffmpeg download from {url} failed: {e:#}");
                errors.push(format!("{e:#}"));
            }
        }
    }
    if !ok {
        let _ = std::fs::remove_file(&archive);
        bail!("ffmpeg download failed: {}", errors.join("; "));
    }
    progress(DownloadStage::Extracting);
    let r = extract(&archive, info, &dir);
    let _ = std::fs::remove_file(&archive);
    r?;
    let _ = std::fs::write(
        dir.join("SOURCE.txt"),
        format!(
            "ffmpeg {} ({} build, {}), downloaded by Voiceolator from:\n{}\nsha256 {}\nffmpeg is a separate program (https://ffmpeg.org). Delete this folder to remove it.\n",
            info.version, info.publisher, info.license, info.urls[0], info.sha256
        ),
    );
    check(&dir.join(exe_name())).context("the downloaded ffmpeg does not run on this system")
}

/// Remove a downloaded ffmpeg.
pub fn remove_downloaded() -> Result<()> {
    let d = managed_dir();
    if d.exists() {
        std::fs::remove_dir_all(&d)?;
    }
    Ok(())
}
