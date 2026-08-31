# Cleanup Bot

A Discord bot that automatically deletes old messages based on a per-channel
retention policy. Attachments can optionally be backed up to cloud storage
before their message is deleted.

## Features

- Per-channel retention policies, configured via slash commands
- Attachment backup to OneDrive or Proton Drive before deletion
- Resumable backup queue — uploads that fail are retried on a schedule

## Requirements

- Rust (Edition 2024)
- Discord bot token with the `GUILD_MESSAGES` and `MESSAGE_CONTENT` intents
- (Optional, for cloud backup) Either:
  - An Azure AD app registration for OneDrive, with the `Files.ReadWrite` and
    `offline_access` delegated scopes, configured as a public client
    (device code flow, personal Microsoft accounts)
  - The [Proton Drive CLI](https://proton.me/drive) (`proton-drive`) installed
    and authenticated on the host

## Configuration

### `.env`

```env
DISCORD_TOKEN=<YOUR_DISCORD_BOT_TOKEN>
```

If using the Proton Drive backend, also set how it stores its session
credentials (see [Proton Drive credentials storage](#proton-drive-credentials-storage)):

```env
PROTON_DRIVE_CREDENTIALS_STORE=pass
```

### `config.toml`

```toml
schedule_interval_seconds = 3600

[retention]
default_policy_days = 90

[media_backup]
download_dir = "./media_backups"

[media_backup.worker]
check_interval_seconds = 60
max_retries = 5

# Optional — omit this table entirely to skip cloud backup and keep
# downloaded attachments on local disk only.
[cloud_backup]
upload_folder = "/discord-backups"
backend = "one_drive"       # or "proton_drive"
client_id = "<AZURE_APP_CLIENT_ID>"  # one_drive backend only
```

| Field                                  | Description                                                                        |
| --------------------------------------- | ------------------------------------------------------------------------------------ |
| `schedule_interval_seconds`             | How often the cleanup scheduler runs                                               |
| `retention.default_policy_days`         | Default message retention, used when a channel doesn't set its own `policy_days`    |
| `media_backup.download_dir`             | Where attachments are downloaded to before being uploaded and/or the message deleted |
| `media_backup.worker.check_interval_seconds` | How often the backup worker checks for pending uploads                        |
| `media_backup.worker.max_retries`       | How many times a failed upload is retried before being left in local storage       |
| `cloud_backup.upload_folder`            | Root folder backups are organized under (backend-independent, e.g. `/discord-backups`); files are further organized by `YYYY/MM/DD` |
| `cloud_backup.backend`                  | `"one_drive"` or `"proton_drive"` — only one backend can be active at a time        |
| `cloud_backup.client_id`                | OneDrive only — the Azure AD app's client ID                                       |

`[channels.<id>]` tables are managed automatically by the `/cleanup enable`
and `/cleanup disable` slash commands and don't need to be hand-edited.

### OneDrive authentication

On first run with `backend = "one_drive"` and no `onedrive_tokens.toml` in
the working directory, the bot starts a device code flow and logs a URL and
code to sign in with. Tokens are then persisted to `onedrive_tokens.toml` and
refreshed automatically.

### Proton Drive authentication

The Proton Drive backend shells out to the `proton-drive` CLI, so it must be
installed and authenticated separately — the bot doesn't drive the login
flow itself:

```bash
proton-drive auth login
```

#### Proton Drive credentials storage

The CLI needs somewhere to persist the session it creates. This is set via
`PROTON_DRIVE_CREDENTIALS_STORE`, and must be the same value used for both
`proton-drive auth login` and whatever runs the bot, since the bot spawns
`proton-drive` as a subprocess and needs to find the same session:

- `keychain` (default) — uses the OS keyring (macOS Keychain, or `libsecret`
  on Linux). Requires a keyring daemon (e.g. `gnome-keyring`) to be running,
  which typically means a logged-in desktop session — usually not available
  on a headless server.
- `pass` — uses the [`pass`](https://www.passwordstore.org/) password
  manager (GPG-backed). Works headlessly with no login session required, as
  long as the GPG key has no passphrase (or `gpg-agent`'s cache is primed) so
  it can decrypt non-interactively. This is the recommended option for a
  systemd-run bot on a headless host.
- `unsafe_file` — stores the session in a plaintext file. Simplest option,
  but credentials aren't encrypted at rest.

## Building

From the workspace root:

```bash
cargo build --release -p cleanup-bot
```

The compiled binary will be at `target/release/cleanup-bot`.

## Deployment

See the [workspace README](../README.md#deployment) for the `install.sh` /
`deploy.sh` workflow used to deploy this bot to a systemd host.
