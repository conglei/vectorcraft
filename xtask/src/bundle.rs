//! `cargo xtask bundle [--app vectorcraft|stitchcraft]`: build the release app and wrap it as
//! `dist/VectorCraft.app` (macOS, the default) or `dist/StitchCraft.app`, with the committed app icon
//! `assets/app-icon/<app>.icns` (regenerate it with `packaging/icons.sh [<app>]`).

use std::path::Path;
use std::process::Command;

/// A desktop app `cargo xtask bundle` wraps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum App {
    VectorCraft,
    /// The embroidery app: VectorCraft's launcher under its own name (`apps/stitchcraft`).
    StitchCraft,
}

impl App {
    /// The app named by `--app <name>` (or `--app=<name>`) in `args`; VectorCraft without one.
    pub fn from_args(args: &[&str]) -> Result<App, String> {
        let mut name = None;
        let mut args = args.iter();
        while let Some(a) = args.next() {
            match a.strip_prefix("--app").map(|rest| (rest, rest.strip_prefix('='))) {
                Some(("", _)) => name = Some(*args.next().ok_or("--app needs vectorcraft or stitchcraft")?),
                Some((_, Some(value))) => name = Some(value),
                _ => return Err(format!("bundle: unknown argument {a} (see cargo xtask --help)")),
            }
        }
        match name.unwrap_or("vectorcraft") {
            "vectorcraft" => Ok(App::VectorCraft),
            "stitchcraft" => Ok(App::StitchCraft),
            other => Err(format!("bundle: unknown app {other} (vectorcraft or stitchcraft)")),
        }
    }

    /// The name of the bundle, its executable and its icon file.
    fn name(self) -> &'static str {
        match self {
            App::VectorCraft => "VectorCraft",
            App::StitchCraft => "StitchCraft",
        }
    }

    /// The Cargo package, which is also the binary's name.
    fn package(self) -> &'static str {
        match self {
            App::VectorCraft => "vectorcraft",
            App::StitchCraft => "stitchcraft",
        }
    }
}

pub fn run(root: &Path, app_kind: App) -> Result<(), String> {
    let (name, package) = (app_kind.name(), app_kind.package());
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(root)
        .args(["build", "--release", "-p", package, "-p", "vectorcraft-cli"])
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("release build failed".into());
    }
    let target = std::env::var("CARGO_TARGET_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| root.join("target"));
    let bundle = root.join(format!("dist/{name}.app"));
    let app = bundle.join("Contents");
    let _ = std::fs::remove_dir_all(&bundle);
    std::fs::create_dir_all(app.join("MacOS")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(app.join("Resources")).map_err(|e| e.to_string())?;
    std::fs::copy(target.join(format!("release/{package}")), app.join(format!("MacOS/{name}"))).map_err(|e| format!("copy app: {e}"))?;
    std::fs::copy(target.join("release/vectorcraft-cli"), app.join("MacOS/vectorcraft-cli")).map_err(|e| format!("copy cli: {e}"))?;
    std::fs::copy(root.join(format!("assets/app-icon/{package}.icns")), app.join(format!("Resources/{name}.icns")))
        .map_err(|e| format!("copy icon: {e}"))?;
    let sha = std::env::var("VECTORCRAFT_BUILD_SHA").unwrap_or_else(|_| "unknown".into());
    let plist = info_plist(env!("CARGO_PKG_VERSION"), &sha);
    let plist = match app_kind {
        App::VectorCraft => plist,
        App::StitchCraft => stitchcraft_plist(&plist)?,
    };
    std::fs::write(app.join("Info.plist"), plist).map_err(|e| e.to_string())?;
    println!("built {}", bundle.display());
    Ok(())
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

    #[test]
    fn bundle_takes_the_app_to_wrap() {
        assert_eq!(App::from_args(&[]), Ok(App::VectorCraft));
        assert_eq!(App::from_args(&["--app", "stitchcraft"]), Ok(App::StitchCraft));
        assert_eq!(App::from_args(&["--app=vectorcraft"]), Ok(App::VectorCraft));
        assert!(App::from_args(&["--app"]).is_err());
        assert!(App::from_args(&["--app", "photo"]).is_err());
        assert!(App::from_args(&["stitchcraft"]).is_err());
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
