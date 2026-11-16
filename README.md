<img src="assets/duocle.png" alt="" width="96" height="96">

# Duocle

Keep your active window sharp while the rest of your desktop softens. Duocle runs in the macOS menu bar or the Windows system tray.

## Download Duocle 1.0 (3)

| Your computer | Download |
| --- | --- |
| Mac with Apple silicon · macOS 14.6+ | [Apple silicon DMG](https://github.com/nipunyatawara-dev/Duocle/releases/download/v1.0.0/Duocle-macos-arm64.dmg) |
| Mac with Intel · macOS 14.6+ | [Intel DMG](https://github.com/nipunyatawara-dev/Duocle/releases/download/v1.0.0/Duocle-macos-intel.dmg) |
| Windows 11 · x64 | [Windows installer](https://github.com/nipunyatawara-dev/Duocle/releases/download/v1.0.0/Duocle-windows-x64-setup.exe) |

### Install on macOS

1. Open the DMG and drag **Duocle** into **Applications**.
2. Open Duocle from Applications, then click its menu bar icon.

The Mac app is ad hoc signed and unnotarized. [Apple’s guidance on opening downloaded apps](https://support.apple.com/en-us/102445)

### Install on Windows

1. Run the installer. On the final screen, choose whether to add Desktop and Start menu shortcuts.
2. Open Duocle and click its icon in the system tray by the clock. Check the hidden icons arrow if needed.

The installer is unsigned. Duocle uses the Microsoft Edge WebView2 Runtime for its settings window. Install the Evergreen Runtime if setup prompts for it. [Microsoft WebView2 download](https://developer.microsoft.com/microsoft-edge/webview2/#download-section)

## Use Duocle

Left-click the Duocle icon to toggle focus, switch modes, ignore the active app, or peek at the desktop. Choose **Duocle settings…** for more controls.

- **Choose a blur.** Deep softens the background evenly. Ambient uses a gentler effect. Adjust blur, tint, and grain in **Focus**.
- **Keep the right windows clear.** Use **App rules** to keep an app’s windows or a group of apps sharp. Ignored apps pause the blur when they come to the foreground.
- **Work across displays.** Choose **Per display** to keep a window clear on each display, or **Global** to follow the active window across your desktop.
- **Peek for a moment.** Shake the cursor horizontally to toggle focus. Hold Shift while you shake for a two-second peek. Keep holding Shift to extend it.
- **Switch windows freely.** The blur follows focus and pauses in macOS Mission Control and Windows Task View. On macOS, click bare wallpaper to clear the blur and select a window to resume it.

### Default shortcuts

| Action | macOS | Windows |
| --- | --- | --- |
| Toggle focus | Control + Option + Command + F | Ctrl + Alt + Shift + F |
| Switch focus mode | Control + Option + Command + M | Ctrl + Alt + Shift + M |
| Toggle the active app’s ignore rule | Control + Option + Command + E | Ctrl + Alt + Shift + E |
| Toggle settings | Control + Option + Command + C | Ctrl + Alt + Shift + C |

Change shortcuts, peek gestures, transitions, or launch at login in **Controls**.

## Screen access

- **macOS** — Standard blur runs in AppKit without Screen Recording or Accessibility permission. Screen Recording is required for the optional **Precise effects** setting, which adds pixel-based blur and monochrome.
- **Windows** — Blur and monochrome use local desktop frames. Protected surfaces and some layered windows may render differently in the overlay.

Duocle processes captured frames in memory and saves or sends none of them.

## Automation

Start the desktop app before sending commands. On macOS, use `/Applications/Duocle.app/Contents/MacOS/duocle`. On Windows, use `Duocle.exe` from the installation folder. In a PowerShell script, use `Start-Process -NoNewWindow -Wait` when you need to wait for the Windows executable.

```sh
duocle status
duocle toggle
duocle mode ambient
duocle set blur 70
duocle peek
duocle --help
```

The installed app also accepts URLs such as `duocle://toggle`, `duocle://peek`, and `duocle://mode?value=ambient`.

## Build from source

Use stable Rust 1.90 or newer. Run `cargo run --locked` to start Duocle and `cargo test --locked` to run its tests.

**macOS.** Install the Xcode Command Line Tools and put `dmgbuild` on your PATH. Build either DMG in `dist/`. [dmgbuild installation](https://dmgbuild.readthedocs.io/en/latest/)

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
./scripts/package-macos-dmg.sh aarch64
./scripts/package-macos-dmg.sh x86_64
```

Run `./scripts/package-macos.sh` for a universal ZIP. Set `DUOCLE_SIGN_IDENTITY` to sign the app with a Developer ID certificate.

**Windows.** Install the Visual Studio C++ Build Tools and Windows SDK, then put NSIS on your PATH.

```powershell
rustup target add x86_64-pc-windows-msvc
.\scripts\package-windows.ps1
```

To build the same x64 installer on macOS, install the Homebrew `mingw-w64` and `makensis` formulae, add the `x86_64-pc-windows-gnu` Rust target, and run `./scripts/package-windows-cross.sh`.

## Feedback and credits

Include your Duocle version, OS version, and display setup in bug reports. [GitHub Issues](https://github.com/nipunyatawara-dev/Duocle/issues)

Duocle was built independently with Monocle as a reference for focus behavior. It uses Material Design 3, Roboto, and Material Symbols. [Monocle changelog](https://trymonocle.app/changelog) · [Bundled font licenses](ui/fonts/)
