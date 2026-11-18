#!/bin/sh
# Instala a roleta no Linux (X11) para o usuário atual:
#   ~/.local/bin/roleta, início com a sessão (~/.config/autostart) e atalho no menu.
# Uso: sh instalar.sh            compila, instala e inicia
#      sh instalar.sh --desinstalar
set -eu

cd "$(dirname "$0")"
BIN="$HOME/.local/bin/roleta"
DADOS="${XDG_DATA_HOME:-$HOME/.local/share}"
AUTO="${XDG_CONFIG_HOME:-$HOME/.config}/autostart/roleta.desktop"
MENU="$DADOS/applications/roleta.desktop"
ICONE="$DADOS/icons/hicolor/scalable/apps/roleta.svg"

parar() {
    pkill -x roleta 2>/dev/null || true
    # espera soltar a trava de instância única
    i=0
    while pgrep -x roleta >/dev/null && [ $i -lt 20 ]; do sleep 0.1; i=$((i + 1)); done
}

if [ "${1:-}" = "--desinstalar" ]; then
    parar
    rm -f "$BIN" "$AUTO" "$MENU" "$ICONE"
    echo "Roleta removida. A configuração continua em ~/.config/roleta (apague se quiser)."
    exit 0
fi

if [ "${XDG_SESSION_TYPE:-x11}" = "wayland" ]; then
    echo "Aviso: a roleta precisa de uma sessão X11; no Wayland o botão lateral não é capturado." >&2
fi

cargo build --release
install -Dm755 target/release/roleta "$BIN"
install -Dm644 assets/icone.svg "$ICONE"

entrada() {
    cat <<EOF
[Desktop Entry]
Type=Application
Name=Roleta
Comment=Menu radial do botão lateral do mouse
Exec=$BIN
Icon=roleta
Terminal=false
Categories=Utility;
StartupNotify=false
EOF
}
mkdir -p "$(dirname "$AUTO")" "$(dirname "$MENU")"
entrada > "$MENU"
{ entrada; echo "X-GNOME-Autostart-enabled=true"; } > "$AUTO"

parar
# solta do terminal: continua rodando depois que ele fechar
if command -v setsid >/dev/null; then
    setsid -f "$BIN" >/dev/null 2>&1 </dev/null
else
    nohup "$BIN" >/dev/null 2>&1 </dev/null &
fi
echo "Roleta instalada em $BIN e iniciada (abre sozinha ao entrar na sessão)."
echo "Opções: ~/.config/roleta/roleta_config.json · log: ~/.local/state/roleta/roleta.log"
