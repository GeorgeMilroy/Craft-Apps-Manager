# ArtCraft Apps Manager

A Windows desktop app, written in Rust, that installs and updates the Craft
apps published at [github.com/storytold](https://github.com/storytold).

- Lists every Craft app with a Windows build, its installed version and the
  latest release.
- Installs and updates apps from their GitHub releases. Downloads go through
  a queue, one at a time, and each file is checked against its published
  SHA-256 before it's installed. The next download starts while the previous
  app is still installing.
- Checks for updates on a schedule (every 4 hours by default) and shows a
  Windows notification when a new version is out.
- Can start when you sign in to Windows. This is off by default. When on, it
  starts minimized in the system tray.
- Closing the window keeps it running in the system tray. Quit from the
  tray menu or from Settings.

## Look

The window is compact (460 × 660 by default) and follows the design of
[getartcraft.com](https://getartcraft.com/apps): the site's light and dark
colours, square corners, hairline rules, Archivo headings, Inter text and
uppercase Geist Mono labels. Settings has a theme switch (System, Light or
Dark), and the half-filled circle in the top bar flips between light and
dark.

The manager's logo is the ArtCraft "A" mark, taken from the ArtCraft app's
own icon (`assets\logo.ico` and `assets\logo.png`). It's used for the window,
the tray, notifications, the exe and the installer.

Each app shows its own icon and colours. The icons are the ones on
getartcraft.com; for apps the site doesn't list yet (SoundCraft, WordCraft,
GridCraft, DeckCraft, CADCraft) they come from the app's repository. They're
embedded in the exe, in `assets\app-icons`, so they show offline. Apps
discovered later get their icon from their repository.

The fonts in `assets\fonts` are Inter, Archivo and Geist Mono, under the SIL
Open Font License (licence texts alongside).

## Supported apps

| App | Repository | What it is |
| --- | --- | --- |
| PhotoCraft | [storytold/photocraft](https://github.com/storytold/photocraft) | Image editing (Photoshop-style) |
| LightCraft | [storytold/lightcraft](https://github.com/storytold/lightcraft) | Photo library and RAW development (Lightroom-style) |
| VectorCraft | [storytold/vectorcraft](https://github.com/storytold/vectorcraft) | Vector illustration (Illustrator-style) |
| DesignCraft | [storytold/designcraft](https://github.com/storytold/designcraft) | Page layout and publishing (InDesign-style) |
| FilmCraft | [storytold/filmcraft](https://github.com/storytold/filmcraft) | Video editing (Premiere Pro-style) |
| EffectCraft | [storytold/effectcraft](https://github.com/storytold/effectcraft) | Motion graphics and VFX (After Effects-style) |
| SoundCraft | [storytold/soundcraft](https://github.com/storytold/soundcraft) | Audio recording and mixing (Pro Tools-style) |
| PdfCraft | [storytold/pdfcraft](https://github.com/storytold/pdfcraft) | PDF viewing and editing (Acrobat-style). Formerly PrintCraft |
| WordCraft | [storytold/wordcraft](https://github.com/storytold/wordcraft) | Word processing (Word-style) |
| GridCraft | [storytold/gridcraft](https://github.com/storytold/gridcraft) | Spreadsheets (Excel-style) |
| DeckCraft | [storytold/deckcraft](https://github.com/storytold/deckcraft) | Presentations (PowerPoint-style) |
| CADCraft | [storytold/cadcraft](https://github.com/storytold/cadcraft) | CAD and drafting (AutoCAD-style) |

ArtCraft itself isn't offered: its logo is the manager's logo. The other
storytold repositories don't publish Windows builds: libraries, services,
test data, forks and older experiments. If a new `*craft` repository starts
publishing Windows releases, the manager adds it automatically.

## How apps are installed

Every app installs from its official MSI into `C:\Program Files\<App>`, for
all users, with a Start menu entry. Windows asks for admin permission each
time. The manager runs `msiexec /passive`, so you see a progress bar but no
wizard. Apps you installed yourself before using the manager are detected
from Windows' list of installed apps.

## Installing the manager

`ArtCraftAppsManager-<version>-x64.msi` installs the manager for all users in
`C:\Program Files\ArtCraft Apps Manager`, with a Start menu shortcut.
Installing a newer MSI upgrades it in place, and it can be removed from
Windows Settings > Apps. The installer closes a running copy first, so no
restart is needed.

Versions before 0.4 were called Craft Apps Manager. The 0.4 installer
replaces them like any other upgrade.

## Building

You need Rust (via [rustup](https://rustup.rs)) and the Visual Studio C++
build tools.

```
cargo build --release
```

This builds `target\release\artcraft-apps-manager.exe`, which also runs without
being installed. Run `cargo test` for the unit tests.

To build the installer, first install the WiX v5 .NET tool (one-time):

```
dotnet tool install --global wix --version 5.0.2
wix extension add -g WixToolset.Util.wixext/5.0.2
```

Then run `installer\build-installer.ps1`. It builds the release exe and
writes `target\installer\ArtCraftAppsManager-<version>-x64.msi`. WiX is pinned
to v5, which is MS-RL licensed. WiX v6 and later require accepting the "Open
Source Maintenance Fee" agreement, which can mean fees for commercial users.

## Download page

`site\` is the download page. Firebase Hosting serves the same content from
seven sites in the `getartcraftupdater` project. The main address, which
search engines and shared links use, is artcraftappsmanager.web.app. The
others are:
- artcraftapps.web.app
- artcraftappsupdater.web.app
- craftappsmanager.web.app
- craftappsupdater.web.app
- getartcraftmanager.web.app
- getartcraftupdater.web.app

`/download` on each one redirects to the newest installer, and
`/latest.json` gives its version, size and SHA-256.

To publish a new version, build the installer, then run:

```
site\build.ps1 -Deploy -KeyFile <path to the service account key>
```

The key is a Firebase service account key for the `getartcraftupdater`
project. Keep it outside this folder. Without `-KeyFile`, the deploy uses the
account the Firebase CLI is signed in with, which needs access to the
project.

That copies the installer into `site\public`, fills in its version, size and
checksum, and deploys to all seven sites. Without `-Deploy` it only
builds. The site list and the Hosting settings they share are in
`site\hosting.json`; the build turns them into `site\firebase.json`, with
one entry per site.

The page uses Firebase Analytics. All seven sites report to the same Google
Analytics stream (`G-WPVYWX5H5V`, the "ArtCraft Apps Manager" web app), set
in `site\src\js\config.js`. Every event, page views included, has a `site`
parameter naming the site it came from; GA's built-in Hostname dimension
shows the same. Local previews don't send anything.

Download button clicks are logged as a `download_click` event, with these
parameters:
- `placement`: which button, `header`, `hero`, `details` or `footer`;
- `version`;
- `file_name`.

Ad blockers stop some of these events, so treat the counts as a lower bound.

## Where things are kept

Program Files is read-only without admin rights, so the manager's data lives
in `C:\ProgramData\ArtCraftAppsManager`, shared by everyone who uses the PC. The
installer gives the Users group write access to it.

| File or folder | Contents |
| --- | --- |
| `settings.json` | Settings |
| `state.json` | Cached release data, and which updates have been announced |
| `downloads\` | Installers. Deleted after installing unless *Keep downloaded installers* is on |
| `logs\` | Windows Installer logs, one per install |

This folder is left in place when the manager is uninstalled. Older
versions kept their data elsewhere, and the manager moves it across on
first start, then removes the old folder:
- 0.1 used `%LOCALAPPDATA%\CraftAppsManager`;
- 0.2 and 0.3 used `C:\ProgramData\CraftAppsManager`.

Starting at sign-in is per Windows user. It uses the
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value
`ArtCraftAppsManager`. Versions before 0.4 used `CraftAppsManager`. The
manager replaces that entry the first time it runs, so if you had it
starting at sign-in, open it once after upgrading.

## GitHub API limits

Release information comes straight from the GitHub API, without signing in.
GitHub allows 60 unauthenticated requests an hour per IP address. A check
uses one request per app (12 today), and looking for new apps adds one a
day. Restarting the app within 30 minutes of a check reuses the cached
results instead of checking again. Normal use stays well within the limit.
If it's ever used up (for example by other tools on the same network), the
app shows when it resets, keeps showing the last results, and checks again
automatically.
