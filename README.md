# Summarizer Bot

A Discord bot that automatically summarizes long messages using a local LLM
(Ollama). When users post messages exceeding a configurable length threshold,
the bot generates concise 2-4 sentence summaries with a playful Franglish-style
introduction.

## Features

- Automatic detection of long messages based on configurable thresholds
- Local LLM inference via Ollama (no cloud API dependencies)
- Playful summary introductions mentioning the original author

## Requirements

- Rust (Edition 2024)
- [Ollama](https://ollama.ai/) running on an accessible
  host with your preferred model
- Discord bot token with `GUILD_MESSAGES` and `MESSAGE_CONTENT` intents

## Configuration

Create a `.env` file with the following variables:

```env
DISCORD_TOKEN=<YOUR_DISCORD_BOT_TOKEN>
LLM_HOST=http://your-ollama-host
LLM_PORT=11434
LLM_MODEL=<YOUR_LLM_MODEL>
MESSAGE_LENGTH_MIN=500
MESSAGE_LENGTH_MAX=2000
```

| Variable                   | Description                                                     |
| -------------------------- | --------------------------------------------------------------- |
| `DISCORD_TOKEN`            | Your Discord bot authentication token                           |
| `LLM_HOST`                 | Ollama server hostname (e.g., `http://localhost`)               |
| `LLM_PORT`                 | Ollama server port (default: `11434`)                           |
| `LLM_MODEL`                | Model to use for summarization (e.g., `llama3.2:3b`)            |
| `MESSAGE_LENGTH_MIN`       | Minimum message length to trigger summarization                 |
| `MESSAGE_LENGTH_MAX`       | Maximum message length to process (longer messages are ignored) |

## Building

Build the release binary on your local machine:

```bash
cargo build --release
```

The compiled binary will be at `target/release/summarizer-bot`.

## Deployment

### 1. Copy files to the server

```bash
scp target/release/summarizer-bot user@server:/opt/summarizer-bot/
scp .env user@server:/opt/summarizer-bot/
```

### 2. Set up the systemd service

Copy and customize the service file:

```bash
scp summarizer-bot.service.example user@server:/tmp/
```

On the server, edit and install the service:

```bash
sudo cp /tmp/summarizer-bot.service.example /etc/systemd/system/summarizer-bot.service
sudo nano /etc/systemd/system/summarizer-bot.service
```

Update the `User` and `WorkingDirectory` fields to match your setup.

### 3. Enable and start the service

```bash
sudo systemctl daemon-reload
sudo systemctl enable summarizer-bot
sudo systemctl start summarizer-bot
```

### 4. Check status

```bash
sudo systemctl status summarizer-bot
sudo journalctl -u summarizer-bot -f
```

## License

MIT License - see [LICENSE](LICENSE) for details.
