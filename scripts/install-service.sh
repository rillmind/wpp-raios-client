#!/usr/bin/env sh
# Compila backend e front e instala como serviço systemd --user.
#
# Os dados ficam em ~/.local/share/wpp-lite/data (config, whatsapp.db, messages.db).
# Para migrar um data/ de desenvolvimento: pare o serviço e copie o diretório para lá.
set -eu
cd "$(dirname "$0")/.."
dest="$HOME/.local/share/wpp-lite"

cargo build --release
(cd frontend && npm ci && npm run build)

mkdir -p "$dest/data"
target="${CARGO_TARGET_DIR:-target}"
install -m 755 "$target/release/wpp-backend" "$dest/wpp-backend"
rm -rf "$dest/frontend"
cp -r frontend/dist "$dest/frontend"

mkdir -p "$HOME/.config/systemd/user"
cp deploy/wpp-lite.service "$HOME/.config/systemd/user/"
systemctl --user daemon-reload
systemctl --user enable --now wpp-lite.service
echo "Instalado. Logs (e QR no primeiro pareamento): journalctl --user -u wpp-lite -f"
echo "Front: http://127.0.0.1:$(grep -E '^port' "$dest/data/config.toml" 2>/dev/null | grep -oE '[0-9]+' || echo 8787)"
