# Feilian Lite Desktop

Tauri 2 and Vue 3 desktop client for Feilian Lite.

## Development

```bash
npm install
npm run tauri dev
```

Linux requires an unlocked Secret Service implementation such as GNOME Keyring
or KWallet; system split-tunnel mode also requires `pkexec`. macOS uses Keychain
and its administrator prompt. Windows uses Credential Manager, UAC, and the
signed Wintun DLL staged during the release build. SOCKS5 mode runs without
elevation on every platform.

The Debian package installs a dedicated Polkit action for
`/usr/bin/feilian-helper`. Direct executable runs do not install that action and
require an already active graphical Polkit agent for system-tunnel authorization.

## Validation

```bash
npm run build
npm run test:e2e
cargo test -p feilian-desktop
cargo clippy -p feilian-desktop --all-targets --no-deps -- -D warnings
```

The real credential-store smoke test is opt-in because headless CI normally has
no unlocked keyring:

```bash
cargo test -p feilian-desktop \
	secret_store::tests::system_secret_store_round_trip -- --ignored --exact
```

With an authenticated desktop profile, the live SOCKS5 lifecycle test exercises
node discovery, helper launch, tunnel startup, and cleanup:

```bash
FEILIAN_LIVE_DATA_DIR="$HOME/.local/share/dev.feilian.lite" \
FEILIAN_HELPER_PATH="$PWD/target/release/feilian-helper" \
cargo test -p feilian-desktop \
	controller::tests::live_socks5_connects_and_cleans_up -- --ignored --exact
```

## Release

```bash
npm run tauri -- build
```

The build compiles and stages `feilian-helper` using Tauri's target-triple
sidecar naming convention, then includes it in the generated platform package.
For cross-compilation, set `FEILIAN_TARGET_TRIPLE` to the same target passed to
Tauri and Cargo.

Native CI builds and tests Linux, macOS, and Windows. macOS artifacts are
unsigned development builds until Developer ID signing and notarization are
configured, so unsigned builds support SOCKS5 but intentionally reject system
tunnel elevation. Windows uses a per-machine installer, downloads Wintun 0.14.1
from the official site, checks its pinned SHA-256, and includes its
redistribution license.

Linux defaults to the reproducible Debian bundle. AppImage remains available as
an explicit target:

```bash
npm run tauri -- build --bundles appimage
```

Older cached `linuxdeploy` GTK plugins may need updating on rolling-release
distributions before that optional bundle can be produced automatically.
