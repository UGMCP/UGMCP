# Installers

Unit Agent already ships as a Tauri 2 desktop app (`com.prysel.unitagent`, version 0.1.0). These commands package that app. They do not replace it.

Finished installers are written to `dist/`. The web bundle Tauri embeds stays in `dist-web/` and is not shown to someone installing the app.

| Command | Where it runs | Output |
| --- | --- | --- |
| `npm run build:mac` | macOS | `dist/Unit-Agent.dmg`, `dist/Unit-Agent-mac.zip` |
| `npm run build:win` | Windows | `dist/Unit-Agent-Setup.exe` |
| `npm run build:all` | the current system | the packages that system can produce |

`build:mac` on Windows or Linux stops with an explanation. `build:win` does the same off Windows. This repository cannot cross-compile the macOS or Windows GUI.

## Installing on macOS

1. Download `Unit-Agent.dmg`.
2. Open it. The window is titled Unit Agent.
3. Drag the Unit Agent icon onto the Applications folder. An arrow on the disk image shows that direction. Both icons are the real application icon and the real Applications alias.
4. Eject the disk image.
5. Open Unit Agent from Launchpad, Finder, Spotlight, or `/Applications`.

The disk image contains the `.app` and the Applications shortcut. Build directories, source files, and `node_modules` are not in it. `Unit-Agent-mac.zip` is the same `.app`, packed with `ditto` so macOS resource forks stay intact.

The window is 660×400. The application sits at (180, 170) and Applications at (480, 170), which matches `src-tauri/icons/dmg-background.png`. `dmg-background@2x.png` is that same layout at 1320×800. `scripts/render-installer-art.py` redraws both backgrounds and the Windows wizard images.

### Architectures

A normal `npm run build:mac` builds for the Mac you are using (Apple silicon or Intel).

A universal binary needs both Rust targets and Xcode:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
UNIT_AGENT_CARGO_TARGET=universal-apple-darwin npm run build:mac
```

The app requires macOS 10.15 or newer. The bundle identifier is `com.prysel.unitagent`. `CFBundleShortVersionString` and `CFBundleVersion` are `0.1.0`.

### Xcode

`macos/UnitAgent.xcodeproj` is an external build system project. Product → Build runs `scripts/xcode-build-mac.sh`, which runs `npm run build:mac`. When Xcode's `ARCHS` contains both `arm64` and `x86_64`, the script requests `universal-apple-darwin`.

`macos/project.yml` is the XcodeGen spec for that project. The checked-in `UnitAgent.xcodeproj` is the file to open. The Xcode project does not rewrite the app in Swift.

### Signing and notarization

No Apple certificate, private key, or password is stored in the repo. Local builds are ad hoc signed so the `.app` opens on that Mac. Hardened runtime is on, using `src-tauri/entitlements.plist` (JIT and unsigned executable memory for the system webview, plus dynamic linker environment variables and library validation disabled for the bundled webview).

To sign and notarize for distribution, install a Developer ID Application certificate in the login keychain and export these variables in the shell that runs the build. Do not commit them.

| Variable | Purpose |
| --- | --- |
| `APPLE_SIGNING_IDENTITY` | Developer ID Application identity, for example `Developer ID Application: Example (TEAMID)`. Overrides `bundle.macOS.signingIdentity`. |
| `APPLE_CERTIFICATE` | Certificate used on CI. When set, Tauri can infer the signing identity. |
| `APPLE_CERTIFICATE_PASSWORD` | Password for that certificate. |
| `APPLE_ID` | Apple ID email used by `notarytool`. Requires `APPLE_PASSWORD` and `APPLE_TEAM_ID`. |
| `APPLE_PASSWORD` | App-specific password for that Apple ID. |
| `APPLE_TEAM_ID` | Ten-character Apple team id. |
| `APPLE_API_KEY` | Notarization API key id, when not using the Apple ID flow. |
| `APPLE_API_ISSUER` | Issuer id for that API key. |
| `APPLE_API_KEY_PATH` | Path to the AuthKey `.p8` file. |
| `APPLE_PROVIDER_SHORT_NAME` | Provider short name when the Apple ID belongs to more than one team. Overrides `bundle.macOS.providerShortName`. |

`bundle.macOS.signingIdentity` can hold the same identity string instead of `APPLE_SIGNING_IDENTITY`. Leave the identity, the provider short name, and every variable above unset when credentials are not available. Ad hoc signing still lets the app open on the build machine.

Tauri notarizes the signed app when the Apple ID trio is set, or when the API key trio is set. `--skip-stapling` can be passed to the Tauri build if stapling should wait. Notarization is required for a Developer ID build distributed outside the Mac App Store.

## Installing on Windows

1. Download `Unit-Agent-Setup.exe`.
2. Open it. The wizard is named Unit Agent and uses the application icon.
3. Choose the install folder if you want a different one. The default for a per-user install is `%LOCALAPPDATA%\Unit Agent`, which does not need an administrator account.
4. Choose the Start menu folder. The installer creates a Start menu shortcut.
5. Install. WebView2 is downloaded only when it is missing, and that step is silent.
6. On the last page, leave **Create desktop shortcut** unchecked or check it. **Run Unit Agent** launches the app.

The same installer upgrades an existing copy: it finds the previous install, offers to install over it, and updates the shortcut. `allowDowngrades` stays on, so an older installer can still replace a newer one after a warning. Uninstall is listed in Windows Settings under Apps. The uninstall page can also delete application data. Shortcuts created by the installer are removed with the app.

### Architectures

On 64-bit Windows:

```bash
rustup target add x86_64-pc-windows-msvc
UNIT_AGENT_CARGO_TARGET=x86_64-pc-windows-msvc npm run build:win
```

Without `UNIT_AGENT_CARGO_TARGET`, the installer matches the machine running the build.

### Code signing

No Windows certificate or password is stored in the repo. The bundle is prepared for signing:

- `bundle.windows.digestAlgorithm` is `sha256`
- `bundle.windows.timestampUrl` is `http://timestamp.digicert.com`

When a certificate is in the Windows certificate store, set `bundle.windows.certificateThumbprint` to its SHA1 thumbprint. Tauri then signs with `signtool`. To sign with another tool, set `bundle.windows.signCommand` and include `%1` where the file path goes (for example `osslsigncode`). Leave the thumbprint and the sign command unset until a certificate exists.

A per-machine install, which defaults to Program Files and requires an administrator, is `bundle.windows.nsis.installMode` set to `perMachine`. `both` lets the person choose current user or all users, and that wizard always asks for administrator rights.

## Version

`package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, and `bundle.macOS.bundleVersion` are all `0.1.0`. The installer script stops if the first three disagree.
