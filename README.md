<div align="center">

# SIO

**Set up Windows the way you like it — once, then repeat it after every reinstall.**

Pick a profile, click once, walk away. SIO installs your apps from winget, Chocolatey
and Scoop, then applies your privacy and performance tweaks — and can undo every one of
them.

Available in English, Russian and Ukrainian.

</div>

---

## Status

Early development, but functionally complete for its own purpose: the app reports the
system it is running on, installs software from the catalog through
winget/Chocolatey/Scoop, applies reversible privacy and performance tweaks, removes
preinstalled bloat, and saves the whole selection as a profile to replay next time.
What is left is packaging and auto-update.

| Milestone | What it delivers | Status |
| --- | --- | --- |
| M0 | Workspace, Tauri + Vue shell, i18n, CI | Done |
| M1 | System dashboard end-to-end | Done |
| M2 | Elevated broker + named-pipe IPC | Done |
| M3 | Package providers, catalog, Apps + Profiles | Done |
| M4 | Tweak engine, revert journal, debloat | Done |
| M5 | Auto-update, release pipeline, v0.1.0 | Done |

**Not yet verified on real hardware.** Every layer is tested — including against the
real registry, the real Service Control Manager and the real `sio-broker.exe` over a
real named pipe — but no software has actually been installed and no tweak actually
applied through the full UAC path. Treat releases as pre-release until that happens,
and try it on a throwaway VM first.

## Where software comes from

SIO installs from **winget, Chocolatey and Scoop** — that is the whole list. The
provider layer is a trait, so other sources can be added, but nothing in this repository
points at cracked-software sites or piracy channels and nothing here is designed to.

Every package identifier in [`catalog/apps.json`](catalog/apps.json) is verified against
the live winget and Chocolatey sources before it lands.

## Design

Four ideas carry most of the weight.

**Package-manager output is not an interface.** `winget search` and `winget list` have
no JSON mode, and their column headers are localized — on a Russian install they come
back in Russian. So providers never parse stdout. Results come from **exit codes**,
inventory comes from `winget export` (a documented, stable schema), and the catalog
stores exact package ids so `--exact` skips searching altogether. Raw output is streamed
to a log pane for humans, never for control flow.

**Reverting needs to know what "nothing" looks like.** Before any tweak writes a value,
its prior state is captured — including the case where the value *did not exist*.
Reverting that means deleting it, not writing a plausible default. A policy key set to
its default is not the same as no policy key, and conflating the two is how tools
silently mangle a system. The prior state goes in a journal; revert replays it inverted.

**The browser engine should not run as administrator.** The UI starts unelevated and
launches no UAC prompt. The first privileged action spawns a small elevated helper that
connects back over a named pipe with a random name and a 256-bit nonce — elevated
processes can't inherit stdio, and a guessable pipe name is squattable. Everything
privileged goes through one trait, so the elevation strategy is swappable at a single
line.

**The catalog is data, not code.** Apps and tweaks live in JSON under `catalog/`. A copy
is compiled in so a freshly-wiped machine with no network still works; a newer copy is
fetched at runtime. Adding an app is a commit, not a release.

## Layout

```
crates/
  sio-core/      domain model, catalog schema, broker protocol — no I/O, no platform deps
  sio-winsys/    every Windows call and every `unsafe` block in the project
  sio-packages/  PackageProvider trait + winget / Chocolatey / Scoop
  sio-tweaks/    tweak engine and revert journal
  sio-broker/    the elevated helper binary
apps/desktop/
  src/           Vue 3 + TypeScript frontend
  src-tauri/     Tauri commands — adapters only, no business logic
catalog/         apps.json, tweaks.json
```

`src-tauri` deliberately contains no logic: each command deserializes, delegates to a
crate, and maps the result. That is what lets `cargo test` cover the interesting parts
without launching a window.

## Installing

Grab the `.exe` (NSIS) or `.msi` from the
[latest release](https://github.com/MyKa322/sio-myka/releases/latest).

SmartScreen will warn on first run: SIO has no Authenticode certificate. That warning is
accurate — it means "nobody paid to vouch for this binary", not "this binary is known to
be fine". Choose *More info* → *Run anyway* only if you trust the build.

In-app updates are a separate matter and *are* verified: each update is signed with the
project's minisign key and rejected if the signature does not match. Check for them under
**Settings → Updates**; SIO never updates itself in the background.

## Building

Requirements: **Rust 1.95+**, **Node 22+**, and the **MSVC C++ build tools** with the
Windows SDK. WebView2 ships with Windows 11.

```bash
npm install --prefix apps/desktop
```

```bash
npm run tauri dev --prefix apps/desktop
```

Build an installer:

```bash
npm run tauri build --prefix apps/desktop
```

### If a dependency fails to compile C code

Some crates build C. If you have several Visual Studio versions installed, `cc` may pick
one whose Windows SDK is not registered and fail with `Cannot open include file:
'stddef.h'`. Build from a Developer PowerShell, or import the environment first:

```bash
cmd /c '"C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat" && cargo build'
```

## Testing

```bash
cargo test --workspace
```

```bash
npm run typecheck --prefix apps/desktop && npm test --prefix apps/desktop
```

Tests that would actually modify the system are `#[ignore]`-gated and never run in CI.
The suite covers revert-plan inversion, catalog validation against the real shipped
files, translation completeness across all three locales, real registry round trips
including `REG_EXPAND_SZ` and `REG_MULTI_SZ`, and the broker handshake — including that
a wrong nonce, a truncated nonce, a version mismatch and a skipped handshake are all
rejected.

`crates/sio-broker/tests/pipe_roundtrip.rs` starts the real `sio-broker.exe` over a real
named pipe and performs real registry work. It spawns the helper *unelevated*, since a
UAC prompt cannot be answered by a test — so everything downstream of elevation is
covered automatically, and only `ShellExecuteExW` itself needs a human. Verify that part
by hand: **Settings → Administrator access → Test administrator access**.

Run real installs and tweaks **on a throwaway VM**, not on a machine you care about.

## Contributing a catalog entry

Add an object to `catalog/apps.json` with an `en`, `ru` and `uk` description and at least
one source. CI enforces both. Verify the identifier first:

```bash
winget show --id Publisher.Package --exact
```

## Licence

MIT.
