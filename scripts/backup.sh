#!/usr/bin/env sh
# Backup de whatsapp.db (sessão) e messages.db (mensagens).
#
# Uso: scripts/backup.sh [diretório-de-dados] [destino]
# Com o `sqlite3` instalado, faz cópia consistente com o serviço rodando
# (`.backup`). Sem ele, pare o serviço antes (systemctl --user stop wpp-lite).
# Mantém os 10 backups mais recentes.
set -eu
data="${1:-${WPP_DATA_DIR:-data}}"
dest="${2:-backups}"
stamp="$(date +%Y%m%d-%H%M%S)"
out="$dest/$stamp"
mkdir -p "$out"
chmod 700 "$dest" "$out"

for db in whatsapp.db messages.db; do
  [ -f "$data/$db" ] || { echo "aviso: $data/$db não existe"; continue; }
  if command -v sqlite3 >/dev/null 2>&1; then
    sqlite3 "$data/$db" ".backup '$out/$db'"
  else
    cp "$data/$db" "$out/$db"
    for ext in -wal -shm; do
      [ -f "$data/$db$ext" ] && cp "$data/$db$ext" "$out/$db$ext"
    done
  fi
done
[ -f "$data/config.toml" ] && cp "$data/config.toml" "$out/"
chmod 600 "$out"/* 2>/dev/null || true
echo "backup em $out"

# Rotação: apaga os mais antigos além dos 10 últimos.
ls -1d "$dest"/*/ 2>/dev/null | sort | head -n -10 | while read -r old; do rm -rf "$old"; done
