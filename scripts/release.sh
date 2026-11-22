#!/bin/sh
# Gera os arquivos da release em dist/ (Linux e Windows sempre juntos):
#   roleta-<v>-linux-x86_64.tar.gz     binário estático (musl), instalar.sh, ícone
#   roleta-<v>-windows-x86_64.zip      Roleta.exe + instalar.ps1 (instala sem Rust)
#   Roleta.exe                         o mesmo .exe, solto, para só abrir
# e os .sha256 de cada um. O .exe é feito com MinGW num container debian:trixie
# (requer podman; cache de build em /hd/cache/roleta-build).
set -eu
cd "$(dirname "$0")/.."
V=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
L="roleta-$V-linux-x86_64"
W="roleta-$V-windows-x86_64"
C="${ROLETA_BUILD_CACHE:-/hd/cache/roleta-build}"
mkdir -p "$C/target-win" "$C/cargo" dist

# --- Linux -------------------------------------------------------------------
cargo build --release --locked -j10 --target x86_64-unknown-linux-musl
B=target/x86_64-unknown-linux-musl/release/roleta
file "$B" | grep -q 'static-pie linked' || { echo "binário Linux não é estático"; exit 1; }
rm -rf "dist/${L:?}" "dist/$L.tar.gz" "dist/$L.tar.gz.sha256"
mkdir -p "dist/$L/assets"
cp "$B" instalar.sh README.md LICENSE "dist/$L/"
cp assets/icone.svg "dist/$L/assets/"
(cd dist && tar czf "$L.tar.gz" "$L" && sha256sum "$L.tar.gz" > "$L.tar.gz.sha256")

# --- Windows -----------------------------------------------------------------
podman run --rm -v "$PWD":/src:ro,Z -v "$C/target-win":/target:Z -v "$C/cargo":/cargo:Z \
    -e CARGO_HOME=/cargo -e CARGO_TARGET_DIR=/target -e RUSTUP_HOME=/cargo/rustup \
    -e CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
    -e DEBIAN_FRONTEND=noninteractive docker.io/library/debian:trixie sh -c '
    set -e
    apt-get update -qq
    apt-get install -y -qq gcc libc6-dev gcc-mingw-w64-x86-64 binutils-mingw-w64-x86-64 curl ca-certificates >/dev/null
    [ -x /cargo/bin/cargo ] || curl -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal --no-modify-path
    /cargo/bin/rustup update -q stable >/dev/null 2>&1 || true
    /cargo/bin/rustup target add x86_64-pc-windows-gnu >/dev/null 2>&1
    /cargo/bin/cargo build --release --locked -j10 --target x86_64-pc-windows-gnu --manifest-path /src/Cargo.toml'
rm -rf "dist/${W:?}" "dist/$W.zip" "dist/$W.zip.sha256" dist/Roleta.exe dist/Roleta.exe.sha256
cp "$C/target-win/x86_64-pc-windows-gnu/release/roleta.exe" dist/Roleta.exe
mkdir -p "dist/$W"
cp dist/Roleta.exe instalar.ps1 README.md LICENSE "dist/$W/"
(cd dist && zip -qr "$W.zip" "$W" && sha256sum "$W.zip" > "$W.zip.sha256" &&
    sha256sum Roleta.exe > Roleta.exe.sha256)

ls -l "dist/$L.tar.gz" "dist/$W.zip" dist/Roleta.exe
cat "dist/$L.tar.gz.sha256" "dist/$W.zip.sha256" dist/Roleta.exe.sha256
