# zuihitsu-discord

A Discord bot that bridges Discord messages, presence, and joins into the zuihitsu platform API.

The connector owns pacing, addressing, typing, and silence decisions at the Discord layer — where
real-time events are available — rather than deferring them to the agent. It is a platform client
that calls the zuihitsu HTTP API; it holds a platform key for `/platform/*`.

## setup

### 1. create a Discord bot application

1. Go to the [Discord developer portal](https://discord.com/developers/applications).
2. Create a new application.
3. Under **Bot**, create a bot user and copy the token.
4. Under **Bot → Privileged Gateway Intents**, enable:
   - **MESSAGE CONTENT Intent** — required to read message text.
   - **SERVER MEMBERS Intent** — required to track member joins and build the present set.
5. Invite the bot to your server with the `bot` scope and appropriate permissions (send messages,
   read message history).

### 2. configure the connector

Create `config.discord.toml`:

```toml
[server]
url = "http://127.0.0.1:7777"
platform_key = "<your platform key>"

[discord]
token = "<your bot token>"

[behavior]
# Channel IDs the bot is authorised to operate in.
# Messages in guild channels not in this list are ignored.
# DMs are always open.
allowed_channels = [123456789012345678]

[pacing]
debounce_ms = 500
typing_refresh_secs = 8
```

### 3. run the connector

```sh
cargo run -p zuihitsu-discord -- --config config.discord.toml
```

The bot connects to Discord and starts forwarding messages. Only messages that mention the bot or
reply to it (in an allowed guild channel) or arrive as DMs are forwarded to the platform API.

## behaviour

- **Addressing**: the bot responds to @mentions, replies to its own messages, and DMs. Messages in
  guild channels that don't mention or reply to the bot are ignored.
- **Pacing**: rapid-fire messages are debounced (500ms default). Only the latest message per channel
  is forwarded when the debounce fires — the agent's buffer carries the rest as context.
- **Typing indicator**: shown only after the agent begins emitting reply tokens (not during
  deliberation), refreshed every 8 seconds, and stopped when the outcome arrives.
- **Context sync**: on first contact with a channel, the connector writes channel metadata and
  laconic guidance to the context memory via `/platform/context`. The context is updated when the
  channel's name or topic changes.
- **Turn mapping**: when a user replies to a mapped message (bot or participant), the connector
  injects a `[turn:<id>]` token into the message text before forwarding to the platform API, so the
  agent can reference the prior turn.
- **Member joins**: on `guild_member_addition`, the connector calls `POST /platform/join` for each
  allowed channel in the guild.

## manual e2e test procedure

1. Start a zuihitsu instance with `config.toml` (model configured, platform key set).
2. Create a Discord bot application, enable MESSAGE CONTENT and GUILD MEMBERS privileged intents.
3. Configure `config.discord.toml` with the bot token, server URL, platform key, and
   allowed channel IDs.
4. Run `cargo run -p zuihitsu-discord -- --config config.discord.toml`.
5. In a Discord channel the bot is authorised in: @mention the bot, verify it responds.
6. Send a message without mentioning the bot, verify it stays silent.
7. DM the bot, verify it responds.
8. Reply to the bot's message, verify the agent can reference the prior turn.
9. Verify typing indicator appears during reply streaming, not during deliberation.
10. Have a user join the server, verify `/platform/join` is called (check server logs).
