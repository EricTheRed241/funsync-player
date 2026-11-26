#!/bin/bash
# Sign the FunSync server binary with a fixed local identity (user, 2026-11-26).
#
# The server lives in ~/Documents, so macOS asks "funsync-server would like to
# access files in your Documents folder" and holds it at launch until someone
# answers. A plain cargo build is ad-hoc signed with a new hash every time, so
# macOS took each rebuild for a new app and asked again. Signed with the same
# certificate and identifier, every rebuild keeps the one Allow.
#
# The certificate is self-signed and lives in a keychain of its own
# (~/Library/Keychains/funsync-signing.keychain-db, made on first use) whose
# password is below: it holds nothing but this local signing key, and a
# keychain of our own lets codesign run without a keychain prompt.
#
# Usage: scripts/server-sign.sh [binary]   (default target/release/funsync-server)
set -euo pipefail
[ "$(uname)" = Darwin ] || exit 0   # macOS privacy prompts only

KC="$HOME/Library/Keychains/funsync-signing.keychain-db"
PASS="funsync-local-signing"
NAME="FunSync Local Signing"
ID="com.funsync.server"
BIN="${1:-$(cd "$(dirname "$0")/.." && pwd)/target/release/funsync-server}"

if [ ! -f "$KC" ]; then
  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT
  cat > "$work/cert.cnf" <<EOF
[req]
distinguished_name=dn
x509_extensions=ext
prompt=no
[dn]
CN=$NAME
[ext]
basicConstraints=critical,CA:false
keyUsage=critical,digitalSignature
extendedKeyUsage=critical,codeSigning
EOF
  # /usr/bin/openssl (LibreSSL) writes a PKCS#12 that `security import` reads.
  /usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 36500 -config "$work/cert.cnf" \
    -keyout "$work/key.pem" -out "$work/cert.pem" 2>/dev/null
  /usr/bin/openssl pkcs12 -export -name "$NAME" -inkey "$work/key.pem" -in "$work/cert.pem" \
    -out "$work/id.p12" -passout pass:"$PASS"
  security create-keychain -p "$PASS" "$KC"
  security set-keychain-settings "$KC"   # no auto-lock timeout
  security import "$work/id.p12" -k "$KC" -P "$PASS" -T /usr/bin/codesign >/dev/null
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$PASS" "$KC" >/dev/null
  echo "Made the signing identity \"$NAME\" in $KC"
fi

security unlock-keychain -p "$PASS" "$KC"
# By hash, from this keychain only: a same-named identity elsewhere (the login
# keychain) made codesign wait on a keychain dialog.
hash=$(security find-identity -p codesigning "$KC" | awk -v n="\"$NAME\"" 'index($0, n) {print $2; exit}')
codesign --force --timestamp=none --keychain "$KC" --sign "$hash" --identifier "$ID" "$BIN"
echo "Signed $BIN as $ID"
