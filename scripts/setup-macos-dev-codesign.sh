#!/usr/bin/env bash
set -euo pipefail

identity="${DIFFY_CODESIGN_IDENTITY:-Diffy Dev}"
keychain="${DIFFY_CODESIGN_KEYCHAIN:-$HOME/Library/Keychains/login.keychain-db}"

repair_key_access() {
  local password="${DIFFY_CODESIGN_KEYCHAIN_PASSWORD:-}"
  if [[ -z "$password" ]]; then
    if [[ -t 0 ]]; then
      printf "login keychain password for codesign access: " >&2
      IFS= read -r -s password
      printf "\n" >&2
    else
      echo "setup-macos-dev-codesign: identity '$identity' exists, but key access may still prompt" >&2
      echo "setup-macos-dev-codesign: rerun in a terminal or set DIFFY_CODESIGN_KEYCHAIN_PASSWORD once" >&2
      return 0
    fi
  fi

  security set-key-partition-list \
    -S apple-tool:,apple:,codesign: \
    -s \
    -t private \
    -k "$password" \
    "$keychain" >/dev/null
}

if security find-identity -v -p codesigning | grep -Fq "\"$identity\""; then
  repair_key_access
  echo "setup-macos-dev-codesign: identity '$identity' already exists; refreshed codesign key access"
  exit 0
fi

tmpdir="$(mktemp -d)"
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT

openssl_config="$tmpdir/openssl.cnf"
key_pem="$tmpdir/diffy-dev.key.pem"
cert_pem="$tmpdir/diffy-dev.cert.pem"
p12="$tmpdir/diffy-dev.p12"
p12_password="$(openssl rand -hex 24)"

cat > "$openssl_config" <<EOF
[ req ]
prompt = no
distinguished_name = dn
x509_extensions = ext

[ dn ]
CN = $identity

[ ext ]
basicConstraints = critical,CA:FALSE
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
subjectKeyIdentifier = hash
EOF

openssl req \
  -new \
  -newkey rsa:2048 \
  -nodes \
  -keyout "$key_pem" \
  -x509 \
  -days 3650 \
  -out "$cert_pem" \
  -config "$openssl_config" >/dev/null 2>&1

openssl pkcs12 \
  -export \
  -inkey "$key_pem" \
  -in "$cert_pem" \
  -name "$identity" \
  -keypbe PBE-SHA1-3DES \
  -certpbe PBE-SHA1-3DES \
  -macalg sha1 \
  -out "$p12" \
  -passout "pass:$p12_password" >/dev/null 2>&1

security import "$p12" \
  -k "$keychain" \
  -P "$p12_password" \
  -T /usr/bin/codesign \
  -T /usr/bin/security >/dev/null

security add-trusted-cert \
  -p codeSign \
  -k "$keychain" \
  "$cert_pem" >/dev/null

repair_key_access

echo "setup-macos-dev-codesign: created code-signing identity '$identity'"
