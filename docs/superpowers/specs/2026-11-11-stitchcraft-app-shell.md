# StitchCraft app shell: design

> **Status:** first pass implemented (STI-22) · **Date:** 2026-11-11 · **Covers:** the StitchCraft desktop app: its binary, name, settings, icon, default workspace, file associations and control port. The embroidery features themselves are in [embroidery-parity.md](../../embroidery-parity.md)

## Goal

The owner decided that embroidery ships as a sibling app, StitchCraft, on VectorCraft's crates
([stitch core design](2026-11-11-embroidery-stitch-core-design.md), open decision 1). Until now the
only way to run it was a renamed copy of `VectorCraft.app`: it said VectorCraft in the window
title, About and Preferences, kept VectorCraft's settings, and its startup update check offered
VectorCraft releases, which would have replaced it. This pass makes StitchCraft a real app: its own
binary and bundle, the same features, its own name everywhere a person sees one.

## Decisions

1. **One launcher, two identities.** `apps/vectorcraft` becomes a library (`vectorcraft_app`,
   `src/lib.rs`, the former `main.rs`) plus a three-line `main.rs`. `apps/stitchcraft` is a second
   binary that calls `vectorcraft_app::run(&STITCHCRAFT)`. An `AppIdentity` names what differs:
   `name`, `lower`, `bundle_id`, `default_control_port`, `update_check`, the three icon PNGs and
   `first_start_panels`. A new crate (`crates/app-shell`) was the alternative; it would have meant
   moving about 40 platform modules (window, GPU, menus, tablets, clipboard, printing) out of the app and
   rewriting their paths, against renaming one file. The app crate is L7 and `cargo xtask layers`
   exempts both apps.
2. **Name.** StitchCraft everywhere a person reads the app's name: window title (with and without a
   document), the app bar title, About (title, heading, brand mark), the macOS app menu (title,
   About, Hide, Quit), the Preferences notes that say when a setting applies, `--help`,
   `--version`, log lines and start-up messages. `vectorcraft_ui_egui::set_app_brand` carries the
   name and the 128 px mark into the UI crate; the few labels that name the app go through one
   `rename`. Menu entries, command labels and docs that name the vector features keep their names.
   The native `.vectorcraft` format keeps its name in file dialogs: it is a file type, not the app.
3. **Menus and tools.** StitchCraft keeps every VectorCraft menu, panel and tool for now: artwork is
   drawn with the vector tools before it is stitched, and nothing is hidden until the owner asks.
   The Embroidery menu (first pass of STI-22) is where the stitch commands live.
4. **Default workspace.** On a first start (no saved preferences) StitchCraft shows the Stitch
   Properties panel (`stitchProps`) and the Stitch Preview panel (`stitchPreview`, STI-29) docked
   open beside the canvas, each in a group of its own, under the Properties group in that order
   (`identity::dock_first_start_panels`, `first_start_panels`); the rest of the layout is
   VectorCraft's default. After that the saved layout comes back as the person left it. An id this
   build does not know is logged and skipped. The home screen heading names StitchCraft.
5. **Home screen.** The same as VectorCraft's (new, open, recent files), under StitchCraft's name
   and mark ("Welcome to StitchCraft"). An embroidery-specific start (hoop sizes, open a machine file) is later work.
6. **Settings.** StitchCraft keeps its own folder: `~/Library/Application Support/StitchCraft`
   (macOS), `%APPDATA%\StitchCraft` (Windows), `$XDG_CONFIG_HOME/stitchcraft` (Linux and the
   BSDs), with everything VectorCraft keeps beside its settings (logs, Data Recovery, Swatches,
   Threads, Plug-ins, Fonts, Launch). **No fallback to VectorCraft's settings on the first run:** a
   VectorCraft layout would hide the embroidery workspace, and the two apps' preferences then drift
   apart anyway. The old DrawCraft settings are VectorCraft's only. Portable mode (a marker beside
   the executable) is VectorCraft's only; StitchCraft has no portable package. The environment
   overrides keep their `VECTORCRAFT_` names in both apps (`VECTORCRAFT_CONFIG_DIR`,
   `VECTORCRAFT_NO_PREFS`, `VECTORCRAFT_CONTROL_PORT`, the automation roots); set
   `VECTORCRAFT_CONFIG_DIR` to different folders to keep two test runs apart.
7. **No update check.** There is no StitchCraft release feed yet. StitchCraft does not check at
   start, and Help › Check for Updates says there is nothing to check
   (`VectorcraftApp::without_release_feed`). When a feed exists, it gets its own `craft-update`
   app entry and `update_check: true`.
8. **File associations (macOS bundle).** `cargo xtask bundle --app stitchcraft` builds
   `dist/StitchCraft.app` with bundle id `ai.storyteller.stitchcraft` and the `StitchCraft` name,
   executable and icon. Its `Info.plist` is made from `packaging/macos/Info.plist.in`: every type
   VectorCraft opens, with `.vectorcraft` documents and `.vctemplate` templates as an alternate
   (VectorCraft owns them; StitchCraft imports their type rather than declaring it), plus
   `.dst`, `.exp` and `.dsb` as **Default**, never Owner (other programs use those names).
   VectorCraft's bundle does not change and still declares no embroidery type. Windows and Linux
   packages for StitchCraft are later work.
9. **Control channel.** `--control <port>` works as in VectorCraft. StitchCraft's conventional port
   is **7989** (VectorCraft's is 7979), so both apps can run side by side under agents; `--help`
   prints it. `--control` still needs the port given explicitly.
10. **Icon.** An original mark drawn for StitchCraft: a needle threaded with a loop of running
    stitches, in the dragon icon's scheme (ink, paper, one field colour: `#2f7d78`).
    `packaging/icons.sh stitchcraft` renders `assets/app-icon/stitchcraft.svg` to the runtime PNGs
    and `stitchcraft.icns`.

## Not in this pass

Hiding vector-only menus or tools; an embroidery home screen; Windows and Linux packages, a
`.desktop` entry and a Windows `.exe` icon for StitchCraft; a release feed; the Stitch Preview panel
itself (STI-29).

## Testing

- `stitchcraft` unit tests: its identity differs from VectorCraft's (bundle id, port, no update
  check), its icons are its own, the window title says StitchCraft, and each first-start panel opens.
- `vectorcraft-ui-egui`: an app without a release feed neither checks at start nor queues a check.
- `xtask`: `--app` parsing, and StitchCraft's plist (names, identifier, embroidery types, imported
  `.vectorcraft` type, a rank for every type) next to VectorCraft's unchanged one.
- By hand: both apps run with `--control`, a screenshot of StitchCraft shows its workspace, and
  `dist/StitchCraft.app` launches with `open`.
