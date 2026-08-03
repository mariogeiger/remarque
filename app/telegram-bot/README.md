# Remarque Telegram service

This headless Rust service is Remarque's private document transport. It accepts
PDFs from one configured Telegram chat, asks the graphical process to open
them, and returns either the immutable source PDF or a flattened annotated
page.

The service is not another tablet UI. It stays alive while Xochitl or Remarque
owns the screen, then hands durable requests to `remarque-tablet`. Its update
offset advances only after the requested effect and Telegram reply succeed.

Configuration is private JSON, normally
`/home/root/remarque/config/telegram.json`:

```json
{"token":"BOT_TOKEN","chat_id":123456789}
```

The service rejects a file with group or world permissions. Never commit this
configuration.

`/next` and `/previous` change the displayed PDF page while preserving an
independent annotation layer for each page. `/close` returns to the persistent
blank page, `/open` restores the last PDF, and `/page` exports whichever page is
currently visible.
