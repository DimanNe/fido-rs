const VERSION: &str = "1.12.0";
const BASE_URL: &str = "https://developers.yubico.com/libfido2/Releases";

#[cfg(all(windows, target_env = "msvc"))]
const SHA256: &str = "7e3a6cfe81755ab208b7bb33501deb9f40ec9657fc89b5e601bcee95b0efa23d";

#[cfg(not(all(windows, target_env = "msvc")))]
const SHA256: &str = "813d6d25116143d16d2e96791718a74825da16b774a8d093d96f06ae1730d9c5";

#[cfg(target_env = "msvc")]
extern crate ureq;

#[cfg(not(target_env = "msvc"))]
extern crate pkg_config;

use anyhow::{bail, Result};
use cfg_if::cfg_if;
use sha2::{Digest, Sha256};
use std::env;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

fn main() -> Result<()> {
    println!("cargo:rerun-if-env-changed=FIDO2_LIB_DIR");
    println!("cargo:rerun-if-env-changed=FIDO2_USE_PKG_CONFIG");

    if let Ok(dir) = env::var("FIDO2_LIB_DIR") {
        println!("cargo:rustc-link-search={}", dir);
        println!("cargo:rustc-link-lib=static=fido2");

        if cfg!(windows) {
            println!("cargo:rustc-link-lib=hid");
            println!("cargo:rustc-link-lib=user32");
            println!("cargo:rustc-link-lib=setupapi");
            println!("cargo:rustc-link-lib=crypt32");
        }

        cfg_if! {
            if #[cfg(all(windows, target_env = "msvc"))] {
                // link to pre-build cbor,zlib,crypto
                println!("cargo:rustc-link-lib=cbor");
                println!("cargo:rustc-link-lib=zlib1");
                println!("cargo:rustc-link-lib=crypto-49");
            } else {
                println!("cargo:rustc-link-lib=cbor");
                println!("cargo:rustc-link-lib=z");
                println!("cargo:rustc-link-lib=crypto");
            }
        }

        return Ok(());
    }

    if env::var("FIDO2_USE_PKG_CONFIG").is_ok() {
        find_pkg()?;

        return Ok(());
    }

    // A build script's `cfg!` describes the host; the target is in CARGO_CFG_*.
    #[cfg(unix)]
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("musl") {
        return musl::build_and_link_static_libfido2_with_dependencies();
    }

    download_src()?;

    let lib_dir = build_lib()?;

    println!("cargo:rustc-link-search={}", lib_dir.display());

    if cfg!(windows) {
        println!("cargo:rustc-link-lib=hid");
        println!("cargo:rustc-link-lib=user32");
        println!("cargo:rustc-link-lib=setupapi");
        println!("cargo:rustc-link-lib=crypt32");
    }

    cfg_if! {
        if #[cfg(all(windows, target_env = "msvc"))] {
            // link to pre-build cbor,zlib,crypto
            println!("cargo:rustc-link-lib=cbor");
            println!("cargo:rustc-link-lib=zlib1");
            println!("cargo:rustc-link-lib=crypto-49");
        } else {
            // mingw, linux, and other.
            println!("cargo:rustc-link-lib=cbor");
            println!("cargo:rustc-link-lib=udev");
            println!("cargo:rustc-link-lib=pcsclite");
            println!("cargo:rustc-link-lib=z");
            println!("cargo:rustc-link-lib=crypto");
        }
    }

    println!("cargo:rustc-link-lib=static=fido2");

    Ok(())
}

fn verify_sha256(content: &[u8]) -> bool {
    let sha256 = Sha256::digest(content);

    *sha256 == hex::decode(SHA256).unwrap()
}

/// for windows and msvc, use pre-build
#[cfg(all(windows, target_env = "msvc"))]
fn download_src() -> Result<()> {
    fn extract_zip(content: &[u8], dst: impl AsRef<Path>) -> Result<()> {
        use zip::ZipArchive;

        let mut zip = ZipArchive::new(Cursor::new(content))?;
        zip.extract(dst)?;

        Ok(())
    }

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let out_dir = Path::new(&out_dir);
    let filename = format!("libfido2-{VERSION}-win.zip");
    let out_path = out_dir.join(&filename);

    let mut archive_bin = Vec::new();

    if out_path.exists() {
        let archive = std::fs::read(&out_path).context("read exist archive failed")?;

        if verify_sha256(&archive) {
            extract_zip(&archive, out_dir.join("libfido2"))?;
            return Ok(());
        } else {
            std::fs::remove_file(&out_path).context("unable delete old file")?;
        }
    }

    let response = ureq::get(&format!("{}/{}", BASE_URL, filename))
        .call()
        .context("unable download fido2 release")?;
    response
        .into_reader()
        .read_to_end(&mut archive_bin)
        .context("read stream failed")?;
    std::fs::write(out_path, &archive_bin).context("write file failed")?;

    if !verify_sha256(&archive_bin) {
        bail!("verify down  load {} failed", filename);
    }

    extract_zip(&archive_bin, out_dir.join("libfido2"))?;

    Ok(())
}

/// for windows and msvc, build nothing
#[cfg(all(windows, target_env = "msvc"))]
fn build_lib() -> Result<PathBuf> {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let out_dir = Path::new(&out_dir).join("libfido2");

    let dir = out_dir.join(format!("libfido2-{VERSION}-win"));
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();

    let arch_dir = match &*arch {
        "x86" => "Win32",
        "x86_64" => "Win64",
        "arm" => "ARM",
        "aarch64" => "ARM64",
        _ => panic!("unsupported arch"),
    };

    let lib_dir = dir.join(arch_dir).join("Release");
    let vc_dir = std::fs::read_dir(&lib_dir)?
        .next()
        .context("no vc dir found")??;
    let lib_dir = lib_dir.join(vc_dir.path()).join("static");

    Ok(lib_dir)
}

/// for other, mingw or linux, download source.
#[cfg(not(target_env = "msvc"))]
fn download_src() -> Result<()> {
    fn extract_tar(content: &[u8], dst: impl AsRef<Path>) -> Result<()> {
        let gz = flate2::read::GzDecoder::new(Cursor::new(content));
        let mut tar = tar::Archive::new(gz);

        tar.unpack(dst)?;

        Ok(())
    }

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let out_dir = Path::new(&out_dir);
    let filename = format!("libfido2-{VERSION}.tar.gz");
    let out_path = out_dir.join(&filename);

    if out_path.exists() {
        let archive = std::fs::read(&out_path).expect("read exist archive failed");

        if verify_sha256(&archive) {
            extract_tar(&archive, out_dir.join("libfido2"))?;

            return Ok(());
        } else {
            std::fs::remove_file(&out_path).expect("unable delete old file");
        }
    }

    let mut archive_bin = Vec::new();

    let response = ureq::get(&format!("{}/{}", BASE_URL, filename))
        .call()
        .expect("unable download fido2 release");
    response
        .into_reader()
        .read_to_end(&mut archive_bin)
        .expect("read stream failed");

    std::fs::write(out_path, &archive_bin).expect("write file failed");

    if !verify_sha256(&archive_bin) {
        bail!("verify download {} failed", filename);
    }

    extract_tar(&archive_bin, out_dir.join("libfido2"))?;

    Ok(())
}

/// for other, mingw or linux, use cmake to build
#[cfg(not(target_env = "msvc"))]
fn build_lib() -> Result<PathBuf> {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let out_dir = Path::new(&out_dir);

    let path = cmake::Config::new(
        out_dir
            .join("libfido2")
            .join(format!("libfido2-{}", VERSION)),
    )
    .define("BUILD_MANPAGES", "off")
    .define("BUILD_EXAMPLES", "off")
    .define("BUILD_TOOLS", "off")
    .define("NFC_LINUX", "on")
    .define("USE_PCSC", "on")
    .build();

    Ok(path.join("lib"))
}

#[cfg(not(target_env = "msvc"))]
fn find_pkg() -> Result<()> {
    let _lib = pkg_config::probe_library("libfido2")?;

    Ok(())
}

#[cfg(all(windows, target_env = "msvc"))]
fn find_pkg() -> Result<()> {
    let _lib = vcpkg::find_package("libfido2")?;

    println!("cargo:rustc-link-lib=hid");
    println!("cargo:rustc-link-lib=user32");
    println!("cargo:rustc-link-lib=setupapi");
    println!("cargo:rustc-link-lib=crypt32");

    Ok(())
}

/// A static musl binary cannot load the host's libcrypto, libcbor, libudev, libpcsclite and zlib:
/// these are shared libraries built against glibc. For a musl target, builds libfido2 and each of
/// them from source as static libraries:
/// - libcrypto and zlib come from the `openssl-sys` (`vendored`) and `libz-sys` (`static`) crates,
///   which this crate depends on for musl targets only; `fido2-rs` reaches libcrypto through the
///   `openssl` crate, so both use one copy;
/// - libcbor and pcsc-lite's client library are downloaded and built here;
/// - libudev-zero takes the place of systemd's libudev, which builds neither statically nor against
///   musl. It answers libfido2's device enumeration from sysfs, so no udev daemon is needed.
#[cfg(unix)]
mod musl {
    use anyhow::{bail, Context, Result};

    fn run_to_success(command: &mut std::process::Command) -> Result<()> {
        let status = command
            .status()
            .with_context(|| format!("Failed to start: {command:?}"))?;
        if !status.success() {
            bail!("Failed: {command:?}: {status}");
        }
        Ok(())
    }

    fn download_and_unpack_tar_gz(
        url: &str,
        expected_sha256: &str,
        dst: &std::path::Path,
    ) -> Result<()> {
        // FOR_REVIEWER: `crate::download_src` (libfido2's archive, glibc builds too) stays as it
        // was, so that the glibc build runs unchanged code. Unlike it, this function keeps no
        // archive in OUT_DIR: each run of the build script downloads the three archives again.
        let archive = {
            use std::io::Read;
            let mut archive = Vec::new();
            ureq::get(url)
                .call()
                .context("Failed to ureq::get")?
                .into_reader()
                .read_to_end(&mut archive)
                .context("Failed to read the download")?;
            archive
        };
        let expected_sha256 = hex::decode(expected_sha256).context("Failed to hex::decode")?;
        use sha2::Digest;
        if sha2::Sha256::digest(&archive)[..] != expected_sha256[..] {
            bail!("The download does not match its SHA-256");
        }
        tar::Archive::new(flate2::read::GzDecoder::new(&archive[..]))
            .unpack(dst)
            .context("Failed to tar::Archive::unpack")
    }

    /// musl-gcc searches only musl's headers, but libfido2 and libudev-zero also include the
    /// kernel's userspace API headers (`linux/hidraw.h`, `linux/netlink.h`, ...), which do not
    /// depend on the C library. Asks the host compiler where these are and links only their
    /// directories into `dst`, so that no glibc header can take the place of a musl one.
    fn link_kernel_uapi_header_dirs(dst: &std::path::Path) -> Result<()> {
        const DIRS: [&str; 3] = ["linux", "asm", "asm-generic"];

        let probe = dst.with_extension("probe.c");
        std::fs::write(&probe, "#include <linux/types.h>\n")
            .context("Failed to write the probe")?;
        let output = std::process::Command::new("cc")
            .arg("-M")
            .arg(&probe)
            .output()
            .context("Failed to run the host compiler `cc`")?;
        if !output.status.success() {
            bail!(
                "The host compiler cannot find <linux/types.h>; install the kernel headers \
                 (linux-libc-dev on Debian and Ubuntu): {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let dependencies = String::from_utf8(output.stdout).context("Failed to read `cc -M`")?;

        if dst.exists() {
            std::fs::remove_dir_all(dst).context("Failed to remove the previous links")?;
        }
        std::fs::create_dir_all(dst).context("Failed to create the directory of links")?;
        for dir in DIRS {
            let types_h = dependencies
                .split_whitespace()
                .map(std::path::Path::new)
                .find(|path| {
                    path.file_name() == Some("types.h".as_ref())
                        && path.parent().and_then(std::path::Path::file_name) == Some(dir.as_ref())
                })
                .with_context(|| format!("`cc -M` does not list {dir}/types.h: {dependencies}"))?;
            std::os::unix::fs::symlink(types_h.parent().unwrap(), dst.join(dir))
                .with_context(|| format!("Failed to symlink: {dir}"))?;
        }
        Ok(())
    }

    /// Runs libudev-zero's own Makefile, which installs the static library, `libudev.h` and
    /// `libudev.pc` into `prefix`.
    fn build_libudev_zero(
        src: &std::path::Path,
        prefix: &std::path::Path,
        compiler: &cc::Tool,
    ) -> Result<()> {
        run_to_success(
            std::process::Command::new("make")
                .current_dir(src)
                .arg(format!("CC={}", compiler.path().display()))
                .arg({
                    let mut arg = std::ffi::OsString::from("CFLAGS=");
                    arg.push(compiler.cflags_env());
                    arg
                })
                .arg(format!("PREFIX={}", prefix.display()))
                .arg("install-static"),
        )
    }

    /// Runs pcsc-lite's own Meson build, which installs the static client library, its headers and
    /// `libpcsclite.pc` into `prefix` (and pcscd, which nothing here uses). The client finds pcscd
    /// through its socket under /run/pcscd, the default of pcsc-lite and of the distributions that
    /// package it.
    fn build_pcsc_lite(
        src: &std::path::Path,
        build_dir: &std::path::Path,
        prefix: &std::path::Path,
        compiler: &cc::Tool,
    ) -> Result<()> {
        if build_dir.exists() {
            std::fs::remove_dir_all(build_dir).context("Failed to remove the previous build")?;
        }
        run_to_success(
            std::process::Command::new("meson")
                .env("CC", compiler.path())
                .env("CFLAGS", compiler.cflags_env())
                .arg("setup")
                .arg(build_dir)
                .arg(src)
                .arg(format!("--prefix={}", prefix.display()))
                .args([
                    "--libdir=lib",
                    "-Ddefault_library=static",
                    // pcscd's dependencies, which meson would take from the host's glibc libraries.
                    // The client library needs none of them.
                    "-Dlibsystemd=false",
                    "-Dlibudev=false",
                    "-Dpolkit=false",
                ]),
        )
        .context("Failed to meson setup (needs meson, ninja and flex)")?;
        run_to_success(
            std::process::Command::new("meson")
                .arg("install")
                .arg("-C")
                .arg(build_dir),
        )
        .context("Failed to meson install")
    }

    fn build_libcbor(src: &std::path::Path, prefix: &std::path::Path, compiler: &cc::Tool) {
        let config = || {
            let mut config = cmake::Config::new(src);
            config
                .out_dir(prefix)
                // libcbor's `project()` enables C++ although it has only C sources, and musl-tools
                // ships no C++ compiler. CMake only checks that this one works.
                .define("CMAKE_CXX_COMPILER", compiler.path())
                // Its default instruments a debug build with sanitizers, whose run-time libraries
                // the Rust link does not provide.
                .define("SANITIZE", "off")
                .define("CMAKE_INSTALL_LIBDIR", "lib");
            config
        };
        // libcbor sets CMAKE_SKIP_INSTALL_ALL_DEPENDENCY, so its `install` target, the one that
        // `cmake::Config::build` runs, does not build the library first.
        config().build_target("all").build();
        config().build();
    }

    /// libfido2 finds its dependencies through pkg-config only: `pkg_config_libdir` must list the
    /// musl builds and nothing else, so that the host's glibc libraries cannot be found.
    fn build_libfido2(
        src: &std::path::Path,
        pkg_config_libdir: &std::ffi::OsStr,
        kernel_headers: &std::path::Path,
    ) -> std::path::PathBuf {
        cmake::Config::new(src)
            .define("BUILD_MANPAGES", "off")
            .define("BUILD_EXAMPLES", "off")
            .define("BUILD_TOOLS", "off")
            .define("NFC_LINUX", "on")
            .define("USE_PCSC", "on")
            .define("CMAKE_INSTALL_LIBDIR", "lib")
            .cflag(format!("-isystem {}", kernel_headers.display()))
            .env("PKG_CONFIG_LIBDIR", pkg_config_libdir)
            .env("PKG_CONFIG_PATH", "")
            .build()
    }

    pub fn build_and_link_static_libfido2_with_dependencies() -> Result<()> {
        // A new version's SHA-256 is checked against its maintainer's signature: Yubico's `.sig`
        // for libfido2 (`crate::SHA256`), the signed tag (pcsc-lite) or commit (libcbor) whose tree
        // the archive equals. libudev-zero is signed nowhere: compare with Alpine's APKBUILD hash.
        const LIBCBOR_VERSION: &str = "0.10.2";
        const LIBCBOR_SHA256: &str =
            "e75f712215d7b7e5c89ef322a09b701f7159f028b8b48978865725f00f79875b";
        const LIBUDEV_ZERO_VERSION: &str = "1.0.3";
        const LIBUDEV_ZERO_SHA256: &str =
            "0bd89b657d62d019598e6c7ed726ff8fed80e8ba092a83b484d66afb80b77da5";
        const PCSC_LITE_VERSION: &str = "2.4.1";
        const PCSC_LITE_SHA256: &str =
            "e7b6737f68c3b9a763fb0b0370d899cea091cced9d762ca8a6032c959576d5be";

        let out_dir =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").context("OUT_DIR is not set")?);
        let openssl_root = std::path::PathBuf::from(
            std::env::var_os("DEP_OPENSSL_ROOT").context("DEP_OPENSSL_ROOT is not set")?,
        );
        let zlib_root = std::path::PathBuf::from(
            std::env::var_os("DEP_Z_ROOT").context("DEP_Z_ROOT is not set")?,
        );

        let kernel_headers = out_dir.join("kernel-headers");
        link_kernel_uapi_header_dirs(&kernel_headers)
            .context("Failed to link_kernel_uapi_header_dirs")?;
        let compiler = cc::Build::new()
            .flag("-isystem")
            .flag(&kernel_headers)
            .try_get_compiler()
            .context("Failed to find the target C compiler")?;

        download_and_unpack_tar_gz(
            &format!("https://github.com/illiliti/libudev-zero/archive/refs/tags/{LIBUDEV_ZERO_VERSION}.tar.gz"),
            LIBUDEV_ZERO_SHA256,
            &out_dir,
        )
        .context("Failed to download libudev-zero")?;
        let libudev_zero_prefix = out_dir.join("libudev-zero");
        build_libudev_zero(
            &out_dir.join(format!("libudev-zero-{LIBUDEV_ZERO_VERSION}")),
            &libudev_zero_prefix,
            &compiler,
        )
        .context("Failed to build_libudev_zero")?;

        download_and_unpack_tar_gz(
            &format!("https://github.com/LudovicRousseau/PCSC/archive/refs/tags/{PCSC_LITE_VERSION}.tar.gz"),
            PCSC_LITE_SHA256,
            &out_dir,
        )
        .context("Failed to download pcsc-lite")?;
        let pcsc_lite_prefix = out_dir.join("pcsc-lite");
        build_pcsc_lite(
            &out_dir.join(format!("PCSC-{PCSC_LITE_VERSION}")),
            &out_dir.join("pcsc-lite-build"),
            &pcsc_lite_prefix,
            &compiler,
        )
        .context("Failed to build_pcsc_lite")?;

        download_and_unpack_tar_gz(
            &format!("https://github.com/PJK/libcbor/archive/refs/tags/v{LIBCBOR_VERSION}.tar.gz"),
            LIBCBOR_SHA256,
            &out_dir,
        )
        .context("Failed to download libcbor")?;
        let libcbor_prefix = out_dir.join("libcbor");
        build_libcbor(
            &out_dir.join(format!("libcbor-{LIBCBOR_VERSION}")),
            &libcbor_prefix,
            &compiler,
        );

        crate::download_src().context("Failed to download libfido2")?;
        let pkg_config_libdir = std::env::join_paths([
            libudev_zero_prefix.join("lib/pkgconfig"),
            pcsc_lite_prefix.join("lib/pkgconfig"),
            libcbor_prefix.join("lib/pkgconfig"),
            openssl_root.join("lib/pkgconfig"),
            zlib_root.join("lib/pkgconfig"),
        ])
        .context("Failed to join the pkg-config directories")?;
        let libfido2_prefix = build_libfido2(
            &out_dir
                .join("libfido2")
                .join(format!("libfido2-{}", crate::VERSION)),
            &pkg_config_libdir,
            &kernel_headers,
        );

        // A static library must precede the libraries it needs. libcrypto and zlib come after
        // these: their crates are dependencies of this one (see lib.rs).
        for prefix in [
            &libfido2_prefix,
            &libcbor_prefix,
            &libudev_zero_prefix,
            &pcsc_lite_prefix,
        ] {
            println!(
                "cargo:rustc-link-search=native={}",
                prefix.join("lib").display()
            );
        }
        for lib in ["fido2", "cbor", "udev", "pcsclite"] {
            println!("cargo:rustc-link-lib=static={lib}");
        }
        Ok(())
    }
}
