//! The ffmpeg that converts WXGF images: `FFMPEG_PATH`, else `ffmpeg` on `PATH`, else the copy `weflow ffmpeg install`
//! put in WeFlow's folder.
//!
//! The install downloads the build the desktop app bundles: npm `ffmpeg-static` 5.3.0, whose binaries are the
//! release `b6.1.1` of github.com/eugeneware/ffmpeg-static (one gzipped executable per platform, plus its license).
//! Every file is checked against the SHA-256 pinned below before it is used, so a mirror of that release
//! (`--base-url`) is as safe as GitHub. Nothing is ever downloaded unless the command is run.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

/// The `ffmpeg-static` release the desktop app's `ffmpeg-static` 5.3.0 downloads (`binary-release-tag`).
pub const RELEASE_TAG: &str = "b6.1.1";
/// Where `ffmpeg-static` downloads its binaries from (the release tag is appended).
pub const DEFAULT_BASE_URL: &str = "https://github.com/eugeneware/ffmpeg-static/releases/download";

/// One platform's files of the release, with their SHA-256 (as GitHub lists them for the release assets).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Build<'a> {
    /// `<platform>-<arch>` in the asset names (`ffmpeg-win32-x64.gz`, `win32-x64.LICENSE`).
    pub asset: &'a str,
    pub gz_sha256: &'a str,
    pub binary_sha256: &'a str,
    pub license_sha256: &'a str,
}

/// The license text of the Windows and Linux builds (GPL-3.0).
const GPL_LICENSE: &str = "8ceb4b9ee5adedde47b31e975c1d90c73ad27b6b165a1dcd80c7c545eb65b903";

const BUILDS: [Build<'static>; 7] = [
    Build {
        asset: "win32-x64",
        gz_sha256: "8883a3dffbd0a16cf4ef95206ea05283f78908dbfb118f73c83f4951dcc06d77",
        binary_sha256: "04e1307997530f9cf2fe35cba2ca7e8875ca91da02f89d6c7243df819c94ad00",
        license_sha256: GPL_LICENSE,
    },
    Build {
        asset: "darwin-x64",
        gz_sha256: "929b375c1182d956c51f7ac25e0b2b0411fb01f6f407aa15c9758efeb4242106",
        binary_sha256: "ebdddc936f61e14049a2d4b549a412b8a40deeff6540e58a9f2a2da9e6b18894",
        license_sha256: "2e1d16c72fd74e12063776371da757322f8b77589386532f4fd8634bde7de1af",
    },
    Build {
        asset: "darwin-arm64",
        gz_sha256: "8923876afa8db5585022d7860ec7e589af192f441c56793971276d450ed3bbfa",
        binary_sha256: "a90e3db6a3fd35f6074b013f948b1aa45b31c6375489d39e572bea3f18336584",
        license_sha256: "cb48bf09a11f5fb576cddb0431c8f5ed0a60157a9ec942adffc13907cbe083f2",
    },
    Build {
        asset: "linux-x64",
        gz_sha256: "bfe8a8fc511530457b528c48d77b5737527b504a3797a9bc4866aeca69c2dffa",
        binary_sha256: "e7e7fb30477f717e6f55f9180a70386c62677ef8a4d4d1a5d948f4098aa3eb99",
        license_sha256: GPL_LICENSE,
    },
    Build {
        asset: "linux-arm64",
        gz_sha256: "754a678672298bc68156adff58aa7385a592c2b30b1d0ae8750c45c915c4bac0",
        binary_sha256: "6bb182d0d75d23028db82e9e4f723ca69b853d055698486e6984ddb2c06fb8ce",
        license_sha256: GPL_LICENSE,
    },
    Build {
        asset: "linux-arm",
        gz_sha256: "64b115a12f0ab77c277e3c418aae8b40ef881e75e746a0e2d066a206b9bc5172",
        binary_sha256: "0afba4a11110e6e402053e0fc14c33a7eb207d7a588688ae87dba471a0f06c71",
        license_sha256: GPL_LICENSE,
    },
    Build {
        asset: "linux-ia32",
        gz_sha256: "169b27c078a8ecedb814cac67afccf15a9868d63e9d74ef86088adefaa500d00",
        binary_sha256: "c6472eb993612db72ca50893a34137ba11173e60a1a4c028d4660a3f755d2490",
        license_sha256: GPL_LICENSE,
    },
];

/// The build for `os` / `arch` (as in `std::env::consts`). The release has no Windows Arm build: Windows 11 on Arm
/// runs the x64 one.
pub fn build_for(os: &str, arch: &str) -> Option<Build<'static>> {
    let asset = match (os, arch) {
        ("windows", "x86_64" | "aarch64") => "win32-x64",
        ("macos", "x86_64") => "darwin-x64",
        ("macos", "aarch64") => "darwin-arm64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("linux", "arm") => "linux-arm",
        ("linux", "x86") => "linux-ia32",
        _ => return None,
    };
    BUILDS.iter().copied().find(|b| b.asset == asset)
}

/// The mirror address `ffmpeg set baseurl` stores: `http://` or `https://`, without a trailing `/`.
pub fn check_base_url(url: &str) -> AppResult<String> {
    let url = url.trim().trim_end_matches('/');
    if (url.starts_with("http://") || url.starts_with("https://")) && url.len() > "https://".len() {
        Ok(url.to_string())
    } else {
        Err(AppError::usage(format!(
            "the ffmpeg download address must start with http:// or https://: {url}"
        )))
    }
}

fn executable_name() -> &'static str {
    if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

/// Folder `ffmpeg install` puts the executable and its license in.
pub fn install_dir(home: &Path) -> PathBuf {
    home.join("ffmpeg").join(RELEASE_TAG)
}

static HOME: OnceLock<PathBuf> = OnceLock::new();
/// The ffmpeg found last, kept for the process: an export converts many images. A missing one is looked for again,
/// so a long-running `serve` picks up an ffmpeg installed meanwhile.
static FOUND: Mutex<Option<(PathBuf, &'static str)>> = Mutex::new(None);

/// WeFlow's folder (where `ffmpeg install` puts ffmpeg); set once when the CLI starts.
pub fn set_home(home: &Path) {
    let _ = HOME.set(home.to_path_buf());
}

/// The ffmpeg to run and where it was found: `FFMPEG_PATH`, `PATH`, `installed`, or `missing` (then plain `ffmpeg`,
/// which fails to start).
pub fn locate() -> (PathBuf, &'static str) {
    if let Some(found) = FOUND.lock().unwrap().clone() {
        return found;
    }
    let found = find(
        std::env::var_os("FFMPEG_PATH"),
        std::env::var_os("PATH"),
        HOME.get().map(|h| install_dir(h)),
    );
    if found.1 != "missing" {
        *FOUND.lock().unwrap() = Some(found.clone());
    }
    found
}

fn find(
    env: Option<std::ffi::OsString>,
    path: Option<std::ffi::OsString>,
    installed: Option<PathBuf>,
) -> (PathBuf, &'static str) {
    if let Some(p) = env.filter(|p| !p.to_string_lossy().trim().is_empty()) {
        return (PathBuf::from(p), "FFMPEG_PATH");
    }
    let name = executable_name();
    if let Some(p) = path
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
    {
        return (p, "PATH");
    }
    if let Some(p) = installed.map(|d| d.join(name)).filter(|p| p.is_file()) {
        return (p, "installed");
    }
    (PathBuf::from("ffmpeg"), "missing")
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn checked(what: &str, bytes: &[u8], expected: &str) -> AppResult<()> {
    let got = sha256_hex(bytes);
    if got == expected {
        Ok(())
    } else {
        Err(AppError::runtime(format!(
            "SHA-256 of {what} does not match: expected {expected}, got {got}"
        )))
    }
}

async fn download(
    client: &reqwest::Client,
    url: &str,
    progress: &dyn Fn(usize, usize),
) -> AppResult<Vec<u8>> {
    let failed =
        |e: &dyn std::fmt::Display| AppError::runtime(format!("download of {url} failed: {e}"));
    let mut resp = client.get(url).send().await.map_err(|e| failed(&e))?;
    if !resp.status().is_success() {
        return Err(failed(&resp.status()));
    }
    let total = resp.content_length().unwrap_or(0) as usize;
    let mut body = Vec::with_capacity(total);
    while let Some(chunk) = resp.chunk().await.map_err(|e| failed(&e))? {
        body.extend_from_slice(&chunk);
        progress(body.len(), total.max(body.len()));
    }
    Ok(body)
}

/// Downloads `build` from `<base_url>/<RELEASE_TAG>/` into `dir`, checking every file, unless `dir` already holds an
/// intact copy (and not `force`). The executable is written beside under another name and renamed into place.
pub async fn install_build(
    dir: &Path,
    base_url: &str,
    build: Build<'_>,
    force: bool,
    progress: &dyn Fn(usize, usize),
) -> AppResult<Value> {
    let target = dir.join(executable_name());
    let license = dir.join("LICENSE");
    let intact = |p: &Path, sha: &str| std::fs::read(p).is_ok_and(|b| sha256_hex(&b) == sha);
    if !force && intact(&target, build.binary_sha256) && intact(&license, build.license_sha256) {
        return Ok(json!({ "path": target, "release": RELEASE_TAG, "downloaded": false }));
    }
    let base = format!("{}/{RELEASE_TAG}", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60))
        .no_gzip()
        .build()
        .map_err(|e| AppError::runtime(e.to_string()))?;
    let gz_url = format!("{base}/ffmpeg-{}.gz", build.asset);
    let gz = download(&client, &gz_url, progress).await?;
    checked(&gz_url, &gz, build.gz_sha256)?;
    let license_url = format!("{base}/{}.LICENSE", build.asset);
    let license_text = download(&client, &license_url, &|_, _| {}).await?;
    checked(&license_url, &license_text, build.license_sha256)?;
    let mut binary = Vec::new();
    flate2::read::GzDecoder::new(gz.as_slice())
        .read_to_end(&mut binary)
        .map_err(|e| AppError::runtime(format!("cannot unpack {gz_url}: {e}")))?;
    checked("the unpacked ffmpeg", &binary, build.binary_sha256)?;

    let io = |p: &Path, e: std::io::Error| {
        AppError::runtime(format!("failed to write {}: {e}", p.display()))
    };
    std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    let partial = dir.join(format!("{}.download", executable_name()));
    // the old copy stays until the new one is complete; a half-written one is not left behind
    let write_partial = || -> std::io::Result<()> {
        std::fs::write(&partial, &binary)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    };
    let placed = write_partial()
        .map_err(|e| io(&partial, e))
        .and_then(|()| std::fs::rename(&partial, &target).map_err(|e| io(&target, e)));
    if let Err(e) = placed {
        let _ = std::fs::remove_file(&partial);
        return Err(e);
    }
    std::fs::write(&license, &license_text).map_err(|e| io(&license, e))?;
    Ok(json!({ "path": target, "release": RELEASE_TAG, "downloaded": true, "source": gz_url }))
}

/// `weflow ffmpeg install`: the build for this computer into [`install_dir`], then a check that it starts.
pub async fn install(home: &Path, base_url: Option<&str>, force: bool) -> AppResult<Value> {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    let build = build_for(os, arch).ok_or_else(|| {
        AppError::runtime(format!("no ffmpeg build of {RELEASE_TAG} for {os}-{arch}"))
    })?;
    let progress = |done: usize, total: usize| {
        crate::output::progress("ffmpeg", "downloading ffmpeg", done, total);
    };
    let mut out = install_build(
        &install_dir(home),
        base_url.unwrap_or(DEFAULT_BASE_URL),
        build,
        force,
        &progress,
    )
    .await?;
    let path = install_dir(home).join(executable_name());
    let version = std::process::Command::new(&path)
        .args(["-hide_banner", "-version"])
        .output()
        .map_err(|e| {
            AppError::runtime(format!(
                "ffmpeg was installed at {} but does not start: {e}",
                path.display()
            ))
        })?;
    let first_line = String::from_utf8_lossy(&version.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    out["version"] = json!(first_line);
    let removed = remove_other_releases(home);
    if !removed.is_empty() {
        out["removedReleases"] = json!(removed);
    }
    // which one WXGF images will use: an ffmpeg on PATH or in FFMPEG_PATH comes first
    *FOUND.lock().unwrap() = None;
    let (used, source) = locate();
    out["used"] = json!({ "path": used, "source": source });
    Ok(out)
}

/// Removes the folders of other releases under `<home>/ffmpeg/` (left by an install of an earlier pinned release);
/// returns their names. A folder in use (an ffmpeg still running from it) stays.
fn remove_other_releases(home: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(home.join("ffmpeg")) else {
        return Vec::new();
    };
    let mut removed: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && e.file_name() != RELEASE_TAG)
        .filter(|e| std::fs::remove_dir_all(e.path()).is_ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    removed.sort();
    removed
}

/// `weflow ffmpeg path`: the ffmpeg WXGF images use and where it was found.
pub fn status() -> Value {
    let (path, source) = locate();
    json!({ "path": path, "source": source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    /// Serves `files` (url path → body) over HTTP on a local port until the test ends; returns the base URL and
    /// the number of requests.
    fn serve(
        files: Vec<(String, Vec<u8>)>,
    ) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                let _ = reader.read_line(&mut line);
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                        break;
                    }
                }
                let mut stream = stream;
                match files.iter().find(|(p, _)| *p == path) {
                    Some((_, body)) => {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(body);
                    }
                    None => {
                        let _ = write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                }
            }
        });
        (base, hits)
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(bytes).unwrap();
        enc.finish().unwrap()
    }

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("weflow-ffmpeg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    struct Release {
        gz: Vec<u8>,
        gz_sha: String,
        binary_sha: String,
        license: Vec<u8>,
        license_sha: String,
    }

    fn release(binary: &[u8]) -> Release {
        let gz = gzip(binary);
        let license = b"GPL".to_vec();
        Release {
            gz_sha: sha256_hex(&gz),
            binary_sha: sha256_hex(binary),
            license_sha: sha256_hex(&license),
            gz,
            license,
        }
    }

    fn build(r: &Release) -> Build<'_> {
        Build {
            asset: "test-x64",
            gz_sha256: &r.gz_sha,
            binary_sha256: &r.binary_sha,
            license_sha256: &r.license_sha,
        }
    }

    fn files(r: &Release, gz: Vec<u8>) -> Vec<(String, Vec<u8>)> {
        vec![
            (format!("/{RELEASE_TAG}/ffmpeg-test-x64.gz"), gz),
            (
                format!("/{RELEASE_TAG}/test-x64.LICENSE"),
                r.license.clone(),
            ),
        ]
    }

    #[tokio::test]
    async fn installs_the_checked_build_and_keeps_an_intact_copy() {
        let binary = b"pretend ffmpeg".repeat(1000);
        let r = release(&binary);
        let (base, hits) = serve(files(&r, r.gz.clone()));
        let dir = temp("install");
        let out = install_build(&dir, &base, build(&r), false, &|_, _| {})
            .await
            .unwrap();
        assert_eq!(out["downloaded"], true);
        let target = dir.join(executable_name());
        assert_eq!(std::fs::read(&target).unwrap(), binary);
        assert_eq!(std::fs::read(dir.join("LICENSE")).unwrap(), b"GPL");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
        let requests = hits.load(std::sync::atomic::Ordering::SeqCst);

        // intact: nothing is downloaded again
        let again = install_build(&dir, &base, build(&r), false, &|_, _| {})
            .await
            .unwrap();
        assert_eq!(again["downloaded"], false);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), requests);

        // damaged, or forced: downloaded again
        std::fs::write(&target, b"broken").unwrap();
        let repaired = install_build(&dir, &base, build(&r), false, &|_, _| {})
            .await
            .unwrap();
        assert_eq!(repaired["downloaded"], true);
        assert_eq!(std::fs::read(&target).unwrap(), binary);
        let forced = install_build(&dir, &base, build(&r), true, &|_, _| {})
            .await
            .unwrap();
        assert_eq!(forced["downloaded"], true);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_file_with_another_checksum_is_not_installed() {
        let r = release(b"the real one");
        // a mirror that serves something else under the same name
        let (base, _) = serve(files(&r, gzip(b"something else")));
        let dir = temp("mismatch");
        let err = install_build(&dir, &base, build(&r), false, &|_, _| {})
            .await
            .unwrap_err();
        assert!(err.message.contains("SHA-256"), "{}", err.message);
        assert!(!dir.join(executable_name()).exists());

        // and a missing file is an error too
        let (empty, _) = serve(Vec::new());
        let err = install_build(&dir, &empty, build(&r), false, &|_, _| {})
            .await
            .unwrap_err();
        assert!(err.message.contains("404"), "{}", err.message);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_base_url_must_be_http_and_loses_its_trailing_slash() {
        assert_eq!(
            check_base_url(" https://registry.npmmirror.com/-/binary/ffmpeg-static/ ").unwrap(),
            "https://registry.npmmirror.com/-/binary/ffmpeg-static"
        );
        assert_eq!(
            check_base_url("http://127.0.0.1:8080").unwrap(),
            "http://127.0.0.1:8080"
        );
        for bad in [
            "",
            "ftp://host/x",
            "mirror.example.com",
            "https://",
            "file:///tmp",
        ] {
            assert!(check_base_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn other_releases_are_removed_and_files_are_kept() {
        let home = temp("releases");
        for dir in ["b6.0", RELEASE_TAG, "b7.0"] {
            std::fs::create_dir_all(home.join("ffmpeg").join(dir)).unwrap();
            std::fs::write(home.join("ffmpeg").join(dir).join("ffmpeg"), b"x").unwrap();
        }
        std::fs::write(home.join("ffmpeg").join("notes.txt"), b"mine").unwrap();
        assert_eq!(remove_other_releases(&home), vec!["b6.0", "b7.0"]);
        assert!(
            install_dir(&home).join("ffmpeg").exists(),
            "the current release stays"
        );
        assert!(
            home.join("ffmpeg").join("notes.txt").exists(),
            "files are not touched"
        );
        assert!(remove_other_releases(&temp("no-releases")).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn a_failed_write_leaves_the_old_copy_and_no_partial_file() {
        let binary = b"pretend ffmpeg".repeat(100);
        let r = release(&binary);
        let (base, _) = serve(files(&r, r.gz.clone()));
        let dir = temp("partial");
        std::fs::create_dir_all(&dir).unwrap();
        // the executable's name is taken by a folder: the rename fails
        std::fs::create_dir_all(dir.join(executable_name()).join("inside")).unwrap();
        let err = install_build(&dir, &base, build(&r), true, &|_, _| {})
            .await
            .unwrap_err();
        assert!(err.message.contains("failed to write"), "{}", err.message);
        assert!(
            dir.join(executable_name()).join("inside").exists(),
            "what was there stays"
        );
        assert!(
            !dir.join(format!("{}.download", executable_name())).exists(),
            "no partial file is left"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_release_target_has_a_build() {
        for (os, arch) in [
            ("windows", "x86_64"),
            ("windows", "aarch64"),
            ("linux", "x86_64"),
            ("linux", "aarch64"),
            ("macos", "x86_64"),
            ("macos", "aarch64"),
        ] {
            assert!(build_for(os, arch).is_some(), "{os}-{arch}");
        }
        assert_eq!(build_for("freebsd", "x86_64"), None);
        assert!(BUILDS
            .iter()
            .all(|b| [b.gz_sha256, b.binary_sha256, b.license_sha256]
                .iter()
                .all(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))));
    }

    #[test]
    fn ffmpeg_path_then_path_then_the_installed_copy() {
        let dir = temp("find");
        let (on_path, installed) = (dir.join("bin"), dir.join("installed"));
        std::fs::create_dir_all(&on_path).unwrap();
        std::fs::create_dir_all(&installed).unwrap();
        let path_var = std::env::join_paths([&on_path]).unwrap();
        assert_eq!(
            find(None, Some(path_var.clone()), Some(installed.clone())).1,
            "missing"
        );
        std::fs::write(installed.join(executable_name()), b"x").unwrap();
        assert_eq!(
            find(None, Some(path_var.clone()), Some(installed.clone())),
            (installed.join(executable_name()), "installed")
        );
        std::fs::write(on_path.join(executable_name()), b"x").unwrap();
        assert_eq!(
            find(None, Some(path_var.clone()), Some(installed.clone())),
            (on_path.join(executable_name()), "PATH")
        );
        assert_eq!(
            find(Some("/opt/ff".into()), Some(path_var), Some(installed)).1,
            "FFMPEG_PATH"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
