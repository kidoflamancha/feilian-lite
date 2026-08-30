# Feilian Lite Desktop

Tauri 2 and Vue 3 desktop client for Feilian Lite.

## Development

```bash
npm install
npm run tauri dev
```

On Linux, an unlocked Secret Service implementation such as GNOME Keyring or
KWallet is required. System split-tunnel mode also requires `pkexec`; SOCKS5
mode runs without elevation.

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

Linux defaults to the reproducible Debian bundle. AppImage remains available as
an explicit target:

```bash
npm run tauri -- build --bundles appimage
```

Older cached `linuxdeploy` GTK plugins may need updating on rolling-release
distributions before that optional bundle can be produced automatically.
