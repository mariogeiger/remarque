# Remarque Telegram service

This headless Rust service is Remarque's private document transport. It accepts
PDFs from one configured Telegram chat, asks the graphical process to import
and open them, lists the tablet library, and exports either the current page or
the complete annotated document.

The service is not another tablet UI. It stays alive while Xochitl or Remarque
owns the screen, then hands durable requests to `remarque-tablet`. Its update
offset advances only after the requested effect and Telegram reply succeed.

Configuration is private JSON, normally
`/home/root/remarque/config/telegram.json`:

```json
{
  "token": "BOT_TOKEN",
  "chat_id": 123456789,
  "relay": {
    "origin": "https://remarque.geiger.ink",
    "owner_token": "LONG_RANDOM_RELAY_CONTROL_TOKEN"
  }
}
```

The service rejects a file with group or world permissions. Never commit this
configuration.

Sending a PDF imports and opens it. `/library` presents inline buttons for all
stored PDFs and blank notebooks. `/export` offers the current page or every page
of the active document. Page changes and blank-page insertion stay in the
tablet UI, where their gestures and state are visible.

`/share` creates a 24-hour collaborative link for the displayed page and
connects the tablet as its black owner. `/shares` lists active capabilities and
provides revocation buttons; `/revoke ID` is the explicit equivalent. The
relay section may be omitted to keep document transport enabled without page
sharing.
