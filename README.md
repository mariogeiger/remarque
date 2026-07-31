# Remarque

Live browser streaming for one reMarkable Paper Pro. The Rust agent reads the
BGRA display allocation from `xochitl` and sends changed `64x64` tiles over a
WebSocket. Nothing is installed on the receiving computer.

```sh
rustup target add aarch64-unknown-linux-musl
cargo build --release --target aarch64-unknown-linux-musl -p remarque-agent
export REMARQUE_HOST=192.168.0.34
scp target/aarch64-unknown-linux-musl/release/remarque-agent \
  root@$REMARQUE_HOST:/home/root/
ssh root@$REMARQUE_HOST /home/root/remarque-agent
```

Open `http://<tablet-ip>:7432`. Press `Command-F` to toggle fullscreen.

Built specifically for firmware `3.27.3.0`. The server has no authentication or
TLS; use it on a trusted LAN. See the [field notes](docs/paper-pro-field-guide.md)
for the device-specific details.
