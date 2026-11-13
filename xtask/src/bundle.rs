//! `cargo xtask bundle [--app vectorcraft|stitchcraft] [--debug]`: build the app and package it for
//! this computer.
//!
//! - macOS: `dist/VectorCraft.app` (the default) or `dist/StitchCraft.app`, with the committed app icon
//!   `assets/app-icon/<app>.icns` (regenerate it with `packaging/icons.sh [<app>]`), signed ad hoc
//!   (or with `MACOS_SIGN_IDENTITY`) inside out and verified with `codesign --verify --strict
//!   --deep`. `--debug` wraps the dev build instead of the release one (a quick check of the
//!   bundle).
//! - Linux: `packaging/linux/package.sh --app <app>` (AppImage, `.deb`, `.rpm`, `.tar.gz`).
//! - Windows: `packaging/windows/package.ps1 -App <app>` (MSI and portable zip).

use std::path::Path;
use std::process::Command;

/// A desktop app `cargo xtask bundle` wraps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum App {
    VectorCraft,
    /// The embroidery app: VectorCraft's launcher under its own name (`apps/stitchcraft`).
    StitchCraft,
}

/// What `cargo xtask bundle` was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub app: App,
    /// macOS: wrap the dev build, not the release one.
    pub debug: bool,
}

impl Options {
    /// The options in `args`: `--app <name>` (or `--app=<name>`), VectorCraft without one, and
    /// `--debug`.
    pub fn from_args(args: &[&str]) -> Result<Options, String> {
        let mut name = None;
        let mut debug = false;
        let mut args = args.iter();
        while let Some(a) = args.next() {
            if *a == "--debug" {
                debug = true;
                continue;
            }
            match a.strip_prefix("--app").map(|rest| (rest, rest.strip_prefix('='))) {
                Some(("", _)) => name = Some(*args.next().ok_or("--app needs vectorcraft or stitchcraft")?),
                Some((_, Some(value))) => name = Some(value),
                _ => return Err(format!("bundle: unknown argument {a} (see cargo xtask --help)")),
            }
        }
        let app = match name.unwrap_or("vectorcraft") {
            "vectorcraft" => App::VectorCraft,
            "stitchcraft" => App::StitchCraft,
            other => return Err(format!("bundle: unknown app {other} (vectorcraft or stitchcraft)")),
        };
        Ok(Options { app, debug })
    }
}

/// The computers `cargo xtask bundle` packages for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    MacOs,
    Windows,
    Linux,
}

impl Host {
    /// The computer this runs on; Linux for the other Unixes (their packaging script is Linux's).
    pub fn current() -> Host {
        if cfg!(target_os = "macos") {
            Host::MacOs
        } else if cfg!(windows) {
            Host::Windows
        } else {
            Host::Linux
        }
    }
}

impl App {
    /// The name of the bundle, its executable and its icon file.
    fn name(self) -> &'static str {
        match self {
            App::VectorCraft => "VectorCraft",
            App::StitchCraft => "StitchCraft",
        }
    }

    /// The name the packaging scripts take for `--app` (`-App`).
    fn id(self) -> &'static str {
        self.package()
    }

    /// The Cargo package, which is also the binary's name.
    fn package(self) -> &'static str {
        match self {
            App::VectorCraft => "vectorcraft",
            App::StitchCraft => "stitchcraft",
        }
    }
}

/// The program and arguments that package `app` on `host` where it isn't a macOS bundle: the
/// platform's packaging script, which builds the release binaries itself.
pub fn package_command(host: Host, app: App) -> Option<(&'static str, Vec<&'static str>)> {
    match host {
        Host::MacOs => None,
        Host::Linux => Some(("packaging/linux/package.sh", vec!["--app", app.id()])),
        Host::Windows => Some(("pwsh", vec!["-NoProfile", "-File", "packaging\\windows\\package.ps1", "-App", app.id()])),
    }
}

pub fn run(root: &Path, options: Options) -> Result<(), String> {
    let app_kind = options.app;
    if let Some((program, args)) = package_command(Host::current(), app_kind) {
        let status = Command::new(program).current_dir(root).args(args).status().map_err(|e| format!("{program}: {e}"))?;
        return if status.success() { Ok(()) } else { Err(format!("{program} failed")) };
    }
    let (name, package) = (app_kind.name(), app_kind.package());
    let profile = if options.debug { "debug" } else { "release" };
    let mut build = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    build.current_dir(root).args(["build", "-p", package, "-p", "vectorcraft-cli"]);
    if !options.debug {
        build.arg("--release");
    }
    let status = build.status().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("{profile} build failed"));
    }
    let target = std::env::var("CARGO_TARGET_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| root.join("target"));
    let bundle = root.join(format!("dist/{name}.app"));
    let app = bundle.join("Contents");
    let _ = std::fs::remove_dir_all(&bundle);
    std::fs::create_dir_all(app.join("MacOS")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(app.join("Resources")).map_err(|e| e.to_string())?;
    std::fs::copy(target.join(format!("{profile}/{package}")), app.join(format!("MacOS/{name}"))).map_err(|e| format!("copy app: {e}"))?;
    std::fs::copy(target.join(format!("{profile}/vectorcraft-cli")), app.join("MacOS/vectorcraft-cli")).map_err(|e| format!("copy cli: {e}"))?;
    std::fs::copy(root.join(format!("assets/app-icon/{package}.icns")), app.join(format!("Resources/{name}.icns")))
        .map_err(|e| format!("copy icon: {e}"))?;
    let sha = std::env::var("VECTORCRAFT_BUILD_SHA").unwrap_or_else(|_| "unknown".into());
    let plist = info_plist(env!("CARGO_PKG_VERSION"), &sha);
    let plist = match app_kind {
        App::VectorCraft => plist,
        App::StitchCraft => stitchcraft_plist(&plist)?,
    };
    std::fs::write(app.join("Info.plist"), plist).map_err(|e| e.to_string())?;
    // Last: a signature seals the files it covers, and nothing may change after it.
    let identity = std::env::var("MACOS_SIGN_IDENTITY").unwrap_or_else(|_| "-".into());
    sign_bundle(&bundle, &[name, "vectorcraft-cli"], &identity, &root.join("packaging/macos/entitlements.plist"))?;
    println!("built and signed ({}) {}", if identity == "-" { "ad hoc" } else { identity.as_str() }, bundle.display());
    Ok(())
}

/// `codesign` over `path` for `identity` (`-`: ad hoc): the hardened runtime with `entitlements`
/// as `packaging/macos/package.sh` signs the release app, and a secure timestamp for a real identity
/// (an ad-hoc signature can't have one).
fn codesign(identity: &str, entitlements: &Path, path: &Path) -> Result<(), String> {
    let timestamp = if identity == "-" { "--timestamp=none" } else { "--timestamp" };
    let mut c = Command::new("codesign");
    c.args(["--force", "--sign", identity, timestamp, "--options", "runtime", "--entitlements"]).arg(entitlements);
    if let Ok(keychain) = std::env::var("MACOS_KEYCHAIN") {
        c.args(["--keychain", keychain.as_str()]);
    }
    let out = c.arg(path).output().map_err(|e| format!("codesign: {e}"))?;
    if out.status.success() { Ok(()) } else { Err(format!("codesign {}: {}", path.display(), String::from_utf8_lossy(&out.stderr).trim())) }
}

/// Sign the app bundle `bundle` inside out, as `package.sh` does: each program in `Contents/MacOS`
/// (`programs`: the main executable and the CLI) first, the bundle itself last, which seals
/// `Info.plist` and `Resources`; then check it with `codesign --verify --strict --deep`. A bundle
/// left as the linker made it, or signed before its `Info.plist` and icon were in place, fails that
/// check.
fn sign_bundle(bundle: &Path, programs: &[&str], identity: &str, entitlements: &Path) -> Result<(), String> {
    for program in programs {
        codesign(identity, entitlements, &bundle.join("Contents/MacOS").join(program))?;
    }
    codesign(identity, entitlements, bundle)?;
    let out = Command::new("codesign")
        .args(["--verify", "--strict", "--deep", "--verbose=2"])
        .arg(bundle)
        .output()
        .map_err(|e| format!("codesign: {e}"))?;
    if out.status.success() { Ok(()) } else { Err(format!("{} does not verify: {}", bundle.display(), String::from_utf8_lossy(&out.stderr).trim())) }
}

/// StitchCraft's `Info.plist`, made from VectorCraft's (`vectorcraft`): its own name, executable,
/// icon and bundle identifier; VectorCraft's document types, but `.vectorcraft` documents and
/// templates as an alternate (VectorCraft owns them, so StitchCraft imports their type instead of
/// declaring it); and the embroidery machine files `.dst`, `.exp` and `.dsb` it opens, as their
/// default app (never the owner: other programs use those names too). An error when the template
/// no longer has what this replaces, so the two cannot drift apart unnoticed.
fn stitchcraft_plist(vectorcraft: &str) -> Result<String, String> {
    const EMBROIDERY: &str = "    <dict>
      <key>CFBundleTypeName</key><string>Embroidery Machine File</string>
      <key>CFBundleTypeExtensions</key><array><string>dst</string><string>exp</string><string>dsb</string></array>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>LSHandlerRank</key><string>Default</string>
    </dict>
  </array>

  <!-- VectorCraft's own format";
    let replacements: [(&str, &str, usize); 5] = [
        ("<string>VectorCraft</string>", "<string>StitchCraft</string>", 5),
        ("<string>ai.storyteller.vectorcraft</string>", "<string>ai.storyteller.stitchcraft</string>", 1),
        ("<string>Owner</string>", "<string>Alternate</string>", 2),
        ("<key>UTExportedTypeDeclarations</key>", "<key>UTImportedTypeDeclarations</key>", 1),
        ("  </array>\n\n  <!-- VectorCraft's own format", EMBROIDERY, 1),
    ];
    let mut plist = vectorcraft.to_owned();
    for (from, to, count) in replacements {
        let found = plist.matches(from).count();
        if found != count {
            return Err(format!("Info.plist.in has {found} of `{from}`, StitchCraft's plist expects {count}"));
        }
        plist = plist.replace(from, to);
    }
    Ok(plist)
}

/// The release `Info.plist` template, which `packaging/macos/package.sh` fills in the same way: the
/// development bundle declares the same document types (Finder opens them with the app).
const INFO_PLIST: &str = include_str!("../../packaging/macos/Info.plist.in");

/// The bundle's `Info.plist`: [`INFO_PLIST`] for `version` and the build commit `sha`.
fn info_plist(version: &str, sha: &str) -> String {
    let short = version.split('-').next().unwrap_or(version);
    INFO_PLIST.replace("@VERSION@", version).replace("@SHORT_VERSION@", short).replace("@BUILD_SHA@", sha)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// File types and wording a raster image editor's packaging carries (lowercase).
    const RASTER_EDITOR: &[&str] = &["pcraft", "psd", "psb", "qoi", "photo", "rastergraphics", "image editor"];

    fn text_files(dir: &Path, out: &mut Vec<(String, String)>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                text_files(&p, out);
            } else if let Ok(t) = std::fs::read_to_string(&p) {
                out.push((p.display().to_string(), t.to_ascii_lowercase()));
            }
        }
    }

    #[test]
    fn packaging_describes_a_vector_app() {
        let mut files = vec![];
        text_files(&crate::root().join("packaging"), &mut files);
        assert!(files.len() > 5, "packaging files found");
        for (path, text) in &files {
            for w in RASTER_EDITOR {
                assert!(!text.contains(w), "{path} mentions `{w}`");
            }
        }
        let read = |p: &str| std::fs::read_to_string(crate::root().join(p)).unwrap();
        let plist = read("packaging/macos/Info.plist.in");
        assert!(plist.contains("<array><string>vectorcraft</string><string>drawcraft</string></array>"), "native format extensions");
        assert!(read("packaging/linux/ai.storyteller.vectorcraft.desktop").contains("Categories=Graphics;2DGraphics;VectorGraphics;"));
        assert!(read("packaging/linux/ai.storyteller.vectorcraft.metainfo.xml.in").contains("<category>VectorGraphics</category>"));
    }

    /// StitchCraft's Linux files: its own ids, names and icons, nothing of VectorCraft's, and a
    /// MIME type declared for each embroidery file the desktop entry lists. (Not run on Linux
    /// itself: `package.sh --app stitchcraft` was not run, only these files read.)
    #[test]
    fn stitchcraft_linux_packaging_is_its_own_app() {
        let read = |p: &str| std::fs::read_to_string(crate::root().join(p)).unwrap();
        let desktop = read("packaging/linux/ai.storyteller.stitchcraft.desktop");
        let metainfo = read("packaging/linux/ai.storyteller.stitchcraft.metainfo.xml.in");
        let mime = read("packaging/linux/ai.storyteller.stitchcraft.mime.xml");
        // Beyond its comments, which say whose types and source these are, they don't name VectorCraft.
        let without_comments = |text: &str| {
            let mut out = String::new();
            let mut rest = text;
            while let Some(start) = rest.find("<!--") {
                out.push_str(&rest[..start]);
                rest = rest[start..].split_once("-->").map_or("", |(_, after)| after);
            }
            out.push_str(rest);
            out.lines()
                .filter(|l| !l.starts_with('#'))
                .collect::<Vec<_>>()
                .join("\n")
                .to_ascii_lowercase()
                .replace("github.com/storytold/vectorcraft", "")
                .replace("application/x-vectorcraft", "")
        };
        for (what, text) in [("desktop entry", &desktop), ("metainfo", &metainfo), ("mime package", &mime)] {
            assert!(!without_comments(text).contains("vectorcraft"), "the {what} names VectorCraft");
        }
        for line in [
            "Name=StitchCraft",
            "Exec=stitchcraft %F",
            "TryExec=stitchcraft",
            "Icon=ai.storyteller.stitchcraft",
            "StartupWMClass=ai.storyteller.stitchcraft",
        ] {
            assert!(desktop.lines().any(|l| l == line), "desktop entry has {line}");
        }
        assert!(
            metainfo.contains("<id>ai.storyteller.stitchcraft</id>")
                && metainfo.contains("<launchable type=\"desktop-id\">ai.storyteller.stitchcraft.desktop</launchable>")
        );
        assert!(metainfo.contains("<binary>stitchcraft</binary>") && metainfo.contains("@VERSION@") && metainfo.contains("@DATE@"));
        // The types the desktop entry lists that are not VectorCraft's or shared-mime-info's are the ones the MIME package declares.
        let listed: Vec<&str> = desktop.lines().find_map(|l| l.strip_prefix("MimeType=")).unwrap().split(';').filter(|t| !t.is_empty()).collect();
        let declared: Vec<&str> = mime.split("<mime-type type=\"").skip(1).filter_map(|b| b.split('"').next()).collect();
        assert_eq!(declared, ["application/x-embroidery-dst", "application/x-embroidery-exp", "application/x-embroidery-dsb"]);
        for t in &declared {
            assert!(listed.contains(t), "{t} is listed");
        }
        for ext in ["dst", "exp", "dsb"] {
            assert!(mime.contains(&format!("<glob pattern=\"*.{ext}\"/>")), "*.{ext}");
        }
        // The icons the packages ship: every size beside the 128 and 256 px ones, and the scalable one.
        for size in ["16x16", "24x24", "32x32", "48x48", "64x64", "128x128", "256x256", "512x512", "scalable"] {
            let ext = if size == "scalable" { "svg" } else { "png" };
            assert!(crate::root().join(format!("assets/app-icon/hicolor/{size}/apps/ai.storyteller.stitchcraft.{ext}")).is_file(), "{size} icon");
        }
        // The packaging script takes the app, installs its own icons and CLI name, and the package
        // description and nfpm file fit it.
        let script = read("packaging/linux/package.sh");
        for want in ["--app)", "APP_ID=ai.storyteller.$APP", "CLI=\"$APP-cli\"", "-name \"$APP_ID.*\"", "/@DESCRIPTION@/"] {
            assert!(script.contains(want), "package.sh has {want}");
        }
        assert!(read("packaging/linux/stitchcraft.description").contains("stitchcraft-cli"));
        assert!(read("packaging/linux/vectorcraft.description").contains("vectorcraft-cli"));
        let nfpm = read("packaging/linux/nfpm.yaml");
        for want in ["name: \"@APP@\"", "/usr/bin/@APP@", "/usr/bin/@CLI@", "@DESCRIPTION@"] {
            assert!(nfpm.contains(want), "nfpm.yaml has {want}");
        }
    }

    /// StitchCraft's Windows package is written from VectorCraft's sources (WiX file, include,
    /// strings, install policy) by `package.ps1`: what it renames has to be in them, and its upgrade
    /// and component ids are its own. (PowerShell and WiX were not run: only the files read.)
    #[test]
    fn stitchcraft_windows_packaging_follows_vectorcraft_sources() {
        let read = |p: &str| std::fs::read_to_string(crate::root().join(p)).unwrap();
        let script = read("packaging/windows/package.ps1");
        let wxs = read("packaging/windows/vectorcraft.wxs");
        let policy = read("packaging/windows/install-policy.json");
        // The ids package.ps1 replaces are the sources' own, and the new ones differ from them and each other.
        let ids = [
            ("601C6437-08AE-4645-ADCC-27F036C2EA2F", "777F2F5E-782C-4B5B-B05D-E0ACB06CFEB1"),
            ("0E8C6C5E-DE4A-4E1D-9DB0-44E8E38C2103", "73E34536-F54B-4492-8B26-ABF426AB6D98"),
            ("498F2D4C-E655-47F6-B406-2D4C449B8D26", "9E2CC842-1B2C-41F9-9CBF-60FE282B7FD0"),
        ];
        for (old, new) in ids {
            assert!(wxs.contains(old), "vectorcraft.wxs has {old}");
            assert!(script.contains(old) && script.contains(new), "package.ps1 maps {old} to {new}");
            assert_ne!(old, new);
            assert!(!wxs.contains(new), "{new} is StitchCraft's alone");
        }
        assert!(policy.contains(ids[0].0), "the policy names the upgrade code");
        for file in ["vectorcraft.wxs", "install-policy.wxi", "vectorcraft.en-us.wxl", "install-policy.json"] {
            assert!(script.contains(file), "package.ps1 converts {file}");
            assert!(
                read(&format!("packaging/windows/{file}")).contains("ectorcraft")
                    || read(&format!("packaging/windows/{file}")).contains("ectorCraft"),
                "{file} names the app"
            );
        }
        // What the renames leave for the CLI and the page.
        assert!(wxs.contains("vectorcraft-cli.exe") && wxs.contains("https://getartcraft.com/apps/vectorcraft"));
        assert!(script.contains("stitchcraft-cli") && script.contains("-PolicyPath $PolicyPath"));
        assert!(read("packaging/windows/check-install-policy.ps1").contains("$PolicyPath"));
        // The exe carries StitchCraft's icon and version info.
        assert!(crate::root().join("assets/app-icon/stitchcraft.ico").is_file());
        let build = read("apps/stitchcraft/build.rs");
        assert!(build.contains("assets/app-icon/stitchcraft.ico") && build.contains("\"StitchCraft\"") && !build.contains("VectorCraft vector"));
    }

    #[test]
    fn bundle_takes_the_app_to_wrap() {
        let app = |args: &[&str]| Options::from_args(args).map(|o| o.app);
        assert_eq!(app(&[]), Ok(App::VectorCraft));
        assert_eq!(app(&["--app", "stitchcraft"]), Ok(App::StitchCraft));
        assert_eq!(app(&["--app=vectorcraft"]), Ok(App::VectorCraft));
        assert!(app(&["--app"]).is_err());
        assert!(app(&["--app", "photo"]).is_err());
        assert!(app(&["stitchcraft"]).is_err());
        // The dev build is asked for with --debug, in any position.
        assert_eq!(Options::from_args(&["--debug", "--app=stitchcraft"]), Ok(Options { app: App::StitchCraft, debug: true }));
        assert_eq!(Options::from_args(&[]), Ok(Options { app: App::VectorCraft, debug: false }));
    }

    /// Off macOS the bundle is the platform's packaging script, for either app.
    #[test]
    fn bundle_runs_the_packaging_script_of_linux_and_windows() {
        assert_eq!(package_command(Host::MacOs, App::StitchCraft), None);
        assert_eq!(package_command(Host::Linux, App::StitchCraft), Some(("packaging/linux/package.sh", vec!["--app", "stitchcraft"])));
        assert_eq!(package_command(Host::Linux, App::VectorCraft), Some(("packaging/linux/package.sh", vec!["--app", "vectorcraft"])));
        let (program, args) = package_command(Host::Windows, App::StitchCraft).unwrap();
        assert_eq!((program, args.last().copied()), ("pwsh", Some("stitchcraft")));
        // The scripts are there, and take the app.
        let read = |p: &str| std::fs::read_to_string(crate::root().join(p)).unwrap();
        assert!(read("packaging/linux/package.sh").contains("--app)"), "package.sh takes --app");
        assert!(read("packaging/windows/package.ps1").contains("$App"), "package.ps1 takes -App");
    }

    /// A bundle as the linker and the bundler leave it (an executable with only the linker's
    /// signature, an `Info.plist`, an icon) is signed so that `codesign --verify --strict --deep`
    /// passes; unsigned, it fails.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_macos_bundle_is_signed_so_that_it_verifies() {
        let dir = std::env::temp_dir().join(format!("vc-bundle-sign-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bundle = dir.join("Tiny.app");
        std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        std::fs::create_dir_all(bundle.join("Contents/Resources")).unwrap();
        std::fs::copy("/usr/bin/true", bundle.join("Contents/MacOS/Tiny")).unwrap();
        std::fs::copy("/usr/bin/false", bundle.join("Contents/MacOS/tiny-cli")).unwrap();
        std::fs::write(bundle.join("Contents/Resources/Tiny.icns"), b"icns").unwrap();
        std::fs::write(bundle.join("Contents/Info.plist"), info_plist("1.2.3", "abc123")).unwrap();
        let verify = || Command::new("codesign").args(["--verify", "--strict", "--deep"]).arg(&bundle).output().unwrap().status.success();
        assert!(!verify(), "unsigned bundle");
        let entitlements = crate::root().join("packaging/macos/entitlements.plist");
        sign_bundle(&bundle, &["Tiny", "tiny-cli"], "-", &entitlements).unwrap();
        assert!(verify(), "signed bundle");
        // Changing a sealed file afterwards breaks it, which sign_bundle reports.
        std::fs::write(bundle.join("Contents/Resources/Tiny.icns"), b"changed").unwrap();
        assert!(!verify());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// StitchCraft's bundle: its own name, executable, icon and identifier; VectorCraft's document
    /// types, with `.vectorcraft` files left to VectorCraft; and the embroidery machine files.
    #[test]
    fn stitchcraft_plist_is_its_own_app() {
        let vectorcraft = info_plist("1.2.3", "abc123");
        let p = stitchcraft_plist(&vectorcraft).unwrap();
        for key in ["CFBundleDisplayName", "CFBundleName", "CFBundleExecutable", "CFBundleIconFile"] {
            assert!(p.contains(&format!("<key>{key}</key>\n  <string>StitchCraft</string>")), "{key}");
        }
        assert!(p.contains("<key>CFBundleIdentifier</key>\n  <string>ai.storyteller.stitchcraft</string>"));
        assert!(!p.contains("<string>VectorCraft</string>") && !p.contains("<string>ai.storyteller.vectorcraft</string>"));
        assert!(p.contains("<array><string>dst</string><string>exp</string><string>dsb</string></array>"), "embroidery files");
        assert!(!p.contains("<string>Owner</string>"), "VectorCraft owns its documents");
        assert!(p.contains("<key>UTImportedTypeDeclarations</key>") && !p.contains("UTExportedTypeDeclarations"));
        assert!(p.contains("<string>ai.storyteller.vectorcraft.document</string>"), "opens .vectorcraft documents");
        assert!(p.contains("<string>svg</string>") && p.contains("<string>1.2.3</string>"), "VectorCraft's types and version");
        // Every document type keeps a role and a rank.
        let types = p.matches("<key>CFBundleTypeName</key>").count();
        assert_eq!(types, vectorcraft.matches("<key>CFBundleTypeName</key>").count() + 1);
        assert_eq!(p.matches("<key>LSHandlerRank</key>").count(), types);
        // VectorCraft's own bundle is unchanged.
        assert_eq!(vectorcraft.matches("<string>Owner</string>").count(), 2);
        assert!(stitchcraft_plist("<plist/>").is_err(), "a template it no longer fits is an error");
    }

    #[test]
    fn dev_bundle_plist_is_the_release_one() {
        let p = info_plist("1.2.3-rc.1", "abc123");
        assert!(p.contains("<string>1.2.3</string>") && p.contains("<string>1.2.3-rc.1</string>"), "{p}");
        assert!(p.contains("<string>abc123</string>"));
        assert!(!p.contains('@'), "every placeholder filled");
        assert!(p.contains("<key>CFBundleDocumentTypes</key>") && p.contains("<string>pdf</string>"), "Finder opens files with it");
    }
}
