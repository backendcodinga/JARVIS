# macOS setup guide (alpha)

Status: **not tested on a second Mac** (waiting for kibong2 to follow this guide on the MacBook Air M2 and confirm in the PR).

This guide explains how to install and start JARVIS on a Mac with Apple silicon. It is written for someone who is not a software developer. If you are a developer and want to run JARVIS from source, see the last section.

## What to expect in the alpha

- **Apple silicon only**: the macOS installer is built for Apple silicon Macs only (such as M1, M2, M3, or M4), not Intel Macs.
- **Typing only today**: voice features (wake word, push-to-talk, speech-to-text, and spoken replies) are not built yet. You talk to JARVIS by typing.
- **No automatic updates**: when a newer build is ready, download and install the new build by hand.
- **Default icon**: the app uses the default Tauri icon until custom artwork is added.

## 1. Download the macOS installer

JARVIS does not publish public release pages yet. Instead, GitHub builds the macOS installer automatically under the `installers` workflow (`.github/workflows/installers.yml`). The build is for **Apple silicon only**, not Intel Macs (`docs/known-limitations.md`).

1. Open the repository on GitHub and click **Actions** (not checked in the app).
2. In the left sidebar, click the **installers** workflow (not checked in the app).
3. Open the latest successful workflow run and scroll down to **Artifacts** (not checked in the app).
4. Download **jarvis-macos** (artifacts are kept for 14 days after a run).
5. Open the downloaded `jarvis-macos.zip` file to extract it. Inside you will find the `.dmg` installer (`*.dmg`) and a `SHA256SUMS.txt` file listing its SHA-256 checksum.
6. Double-click the `.dmg` file and drag **jarvis** / **JARVIS** into your **Applications** folder (not checked in the app).

## 2. Open it the first time (Gatekeeper and Open Anyway)

Because JARVIS is an early alpha built without a paid Apple Developer membership, the macOS app is ad-hoc signed (`signingIdentity: "-"`) and not notarised (decision D-13, spec §6.9, `docs/known-limitations.md`). The first time you open it on a Mac that did not build the app, macOS Gatekeeper blocks it.

### On macOS 15 Sequoia and newer (`Open Anyway`)

Since macOS 15 Sequoia, Control-clicking the app no longer bypasses Gatekeeper (spec §6.9):

1. Open **Applications** and double-click **jarvis** / **JARVIS**, then close the macOS warning dialog (not checked in the app).
2. Open **System Settings** and click **Privacy & Security** (or **Privacy and Security** — not checked in the app).
3. Scroll down to the **Security** section and click **Open Anyway** next to the message about `jarvis` (not checked in the app).
4. Click **Open** (or **Open Anyway** — not checked in the app) in the confirmation dialog and enter your Mac password or use Touch ID if prompted (not checked in the app).

### On earlier macOS versions (`right-click, Open`)

On earlier versions of macOS (`docs/known-limitations.md`), you can also:

1. In **Applications**, right-click (or Control-click) **jarvis** / **JARVIS** and choose **Open** (not checked in the app).
2. Click **Open** in the confirmation dialog (not checked in the app).

After you approve the app once, it opens normally from **Applications** until you replace it with a newer build.

## 3. Permissions macOS asks for

- **Keychain (your API key and Spotify sign-in)**:
  - JARVIS stores your Anthropic API key and your Spotify sign-in refresh token in the **macOS Keychain** (`docs/privacy.md`).
  - macOS may ask you to allow access to the Keychain item (exact prompt wording not checked in the app). Because an ad-hoc signature changes whenever the app is rebuilt, installing a newer build can make macOS ask again (spec §6.9).
- **Automation (controlling Spotify and Music)**:
  - On macOS, music commands such as `pause`, `play`, `next song`, and `what's playing` control the **Spotify** and **Music** apps through macOS (`docs/known-limitations.md`, `docs/privacy.md`).
  - The first time you use a music command for either app, macOS asks once for that app and shows the message from `src-tauri/Info.plist`:
    > "JARVIS asks Spotify and Music to pause, skip and say what is playing, only when you tell it to."
  - To change your answer later (for example, if you said no the first time), open **System Settings** → **Privacy & Security** (or **Privacy and Security**) → **Automation**, find **jarvis** / **JARVIS**, and switch **Spotify** or **Music** on or off (`docs/known-limitations.md`, not checked in the app).
  - Playing music by name (`play <words>`) needs a one-time Spotify setup; see [`docs/known-limitations.md`](known-limitations.md).
- **Microphone**:
  - The microphone is **not used today** because voice input is not built yet (`docs/known-limitations.md`).

## 4. Start JARVIS

1. Open **jarvis** / **JARVIS** from your **Applications** folder (not checked in the app).
2. The main **JARVIS** window opens at 1280×720. Compact orb mode, the tray icon, and starting automatically at login are not built yet (`README.md`).
3. Next, follow [`docs/first-run.md`](first-run.md) to add your API key and try your first commands.

---

## Run from source (for developers)

If you are a developer working on the repository, you need Git, Node.js, pnpm, a Rust toolchain, and the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for macOS.

From the repository folder, run:

```bash
pnpm install
pnpm tauri dev
```

### Optional terminal shortcuts: `jarvis` and `jarvis-mock`

You can install two helper commands so you can start JARVIS from any folder. From the repository folder, run:

```bash
scripts/install-shortcuts.sh
```

This links `scripts/jarvis` into `~/.local/bin` under both names (`jarvis` and `jarvis-mock`) and adds `~/.local/bin` to your `PATH` in your shell startup file (`~/.zshrc` for `zsh`, `~/.bash_profile` for `bash` on macOS, or `~/.profile` otherwise) if it is not there yet. If your shell startup file has an old `alias jarvis=...` line, the installer warns you because an alias takes priority over the command.

Open a **new** Terminal window and, from any folder, type:

| Command       | What it starts                                                                       |
| ------------- | ------------------------------------------------------------------------------------ |
| `jarvis`      | The real app (`pnpm tauri dev`, needs your API key in Settings)                      |
| `jarvis-mock` | The mock backend (`VITE_TRANSPORT=mock`): fake replies, scenario buttons, MOCK badge |

Before starting, each command updates the checkout (`git pull --quiet --ff-only`), but only when you are on `main` with no local changes. On another branch or with local changes, it leaves your checkout alone and says so. Set `JARVIS_NO_UPDATE=1` to skip the update check.

To remove the shortcuts later, run:

```bash
scripts/install-shortcuts.sh --uninstall
```
