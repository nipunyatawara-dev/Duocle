<img src="assets/duocle.png" alt="" width="96" height="96">

# Duocle

**Keep your active window clear and soften the rest of your desktop.**

Duocle is a desktop focus utility for macOS and Windows. It follows your foreground window as you switch apps and applies a customizable blur and tint around it. Find it in the macOS menu bar or the Windows system tray.

[Install](#install) · [Using Duocle](#using-duocle) · [Build from source](#build-from-source)

## Install

Follow the [build instructions](#build-from-source) to create the package for your platform in `dist/`.

| Platform | Setup |
| --- | --- |
| macOS 14.6+ · Apple silicon and Intel | Extract `Duocle-macos-universal.zip`, move `Duocle.app` to Applications, and open it. |
| Windows 11 · x64 | Run `Duocle-windows-x64-setup.exe`, then open Duocle from the Start menu. |

By default, the macOS build is ad hoc signed and unnotarized. The Windows installer is unsigned and uses the Microsoft Edge WebView2 Evergreen Runtime. If the installer says the runtime is missing, install it and rerun setup. [Microsoft WebView2 Runtime](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution)

## Using Duocle

Focus is enabled by default. Click the Duocle icon to open settings, or right-click it to toggle focus, switch modes, peek, or quit.

- **Focus** — Use **Deep** to soften the background uniformly or **Ambient** for a gentler effect. Adjust blur, tint, and grain. For tint-only dimming, set blur, grain, and monochrome to zero. Choose **Per display** to keep a window clear on each display, or **Global** to follow the foreground window across displays.
- **App rules** — Keep all windows from the active app clear, or group apps you use together. Ignored apps stay clear and pause the effect when they come to the foreground.
- **Controls** — Shake the cursor horizontally to toggle focus. Hold Shift and shake to peek for at least two seconds. Keep Shift held to extend the peek. This page also lets you configure corner peek, shortcuts, transitions, and launch at login.

### Default shortcuts

| Action | macOS | Windows |
| --- | --- | --- |
| Toggle focus | Control + Option + Command + F | Ctrl + Alt + Shift + F |
| Switch focus mode | Control + Option + Command + M | Ctrl + Alt + Shift + M |
| Toggle the foreground app's ignore rule | Control + Option + Command + E | Ctrl + Alt + Shift + E |
| Toggle settings | Control + Option + Command + C | Ctrl + Alt + Shift + C |

## Screen access

On macOS, standard blur uses AppKit's compositor and works without Screen Recording or Accessibility access. Enable **Precise effects** and grant Screen Recording permission to control the pixel blur radius or use monochrome.

Windows uses Win32 GDI desktop frames for blur and monochrome. Layered windows and protected hardware surfaces can appear incomplete in those frames. Duocle processes captured frames locally in memory without saving or transmitting them.

## Automation

Start the desktop app before sending CLI commands. Use `/Applications/Duocle.app/Contents/MacOS/duocle` on macOS or `Duocle.exe` from the Windows installation folder. Replace `duocle` below with that path if the executable is outside your shell's path. In PowerShell scripts that need to wait for the Windows executable, use `Start-Process -NoNewWindow -Wait`.

```sh
duocle status
duocle toggle
duocle mode ambient
duocle set blur 70
duocle set app_wide true
duocle peek
duocle --help
```

The installed app also handles URLs such as `duocle://toggle`, `duocle://peek`, `duocle://mode?value=ambient`, and `duocle://tint?value=%23233550`. Encode a leading `#` in a tint as `%23`.

## Build from source

Use stable Rust 1.90 or newer. On a supported OS, run `cargo run --locked` to start Duocle and `cargo test --locked` to run its tests.

**macOS** requires the Xcode Command Line Tools. The script produces `dist/Duocle-macos-universal.zip`. Set `DUOCLE_SIGN_IDENTITY` to use a Developer ID certificate for signing.

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
./scripts/package-macos.sh
```

**Windows** requires the Visual Studio C++ Build Tools, the Windows SDK, and NSIS on your path. The script produces `dist/Duocle-windows-x64-setup.exe`.

```powershell
rustup target add x86_64-pc-windows-msvc
.\scripts\package-windows.ps1
```

To cross-package the Windows installer from macOS, install Homebrew's `mingw-w64` and `makensis` formulae, add the `x86_64-pc-windows-gnu` Rust target, then run `./scripts/package-windows-cross.sh`.

## Credits

Duocle was developed independently, with Monocle 3.5.1 as a reference for focus behavior. Its settings use Material Design 3, with Roboto and Material Symbols bundled locally. [Monocle changelog](https://trymonocle.app/changelog) · [Bundled font licenses](ui/fonts/)

## Feedback

Include your OS version and display setup when reporting a bug. [GitHub Issues](https://github.com/nipunyatawara-dev/Duocle/issues)
