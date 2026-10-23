# Oxitailr: open work

What's known to be left, as of 0.4.1 (2026-10-23). The full history is in
[CHANGELOG.md](CHANGELOG.md); the January egui-era checklist that lived here is gone with
the egui app.

## Desktop app

- **Finish the Windows check** ([docs/HANDOFF-desktop-windows.md](docs/HANDOFF-desktop-windows.md)).
  It installs and tails on a real Windows Server (2026-10-23). Still to try: a single click
  on the tray icon shows and hides the window; an SSH source with a saved password (Windows
  Credential Manager); uninstall.
- **A Linux package** (`.deb` or AppImage) in the release; today Linux users build it.
- **Self-update** with consent, from a signed release feed.
- **Signing** the Windows installer, or a written decision not to (SmartScreen warns on an
  unsigned one).
- **Resource budgets:** measure memory and CPU over a long run and write the numbers down
  as the ones to hold.

## Both apps

- In-app editors for filter presets, highlight rules and alerts (today: the config file).
- A password option in the terminal app's SSH form (today: key auth only).

## Ideas, not decided

- An agent mode: the engine as a background service (NSSM on Windows, systemd on Linux)
  that fires alerts with nobody signed in.
