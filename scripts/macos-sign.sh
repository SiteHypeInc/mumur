#!/usr/bin/env bash
# Re-sign the macOS build with Murmur's stable code-signing certificate and
# rebuild the DMG around it.
#
# Why: ad-hoc signatures change every build, and macOS ties Accessibility /
# Input Monitoring / Microphone grants to the signature, so each update
# silently lost them. Signed with one certificate, the app's designated
# requirement becomes `identifier "app.murmur.dictation" and certificate leaf
# = H"…"`, which stays the same across builds: users grant once.
#
# Needs repository secrets MURMUR_SIGNING_P12_BASE64 and
# MURMUR_SIGNING_P12_PASSWORD (a self-signed "Code Signing" certificate).
# Without them it leaves the ad-hoc build untouched.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ENTITLEMENTS="$ROOT/app/src-tauri/Entitlements.plist"

if [[ -z "${MURMUR_SIGNING_P12_BASE64:-}" || -z "${MURMUR_SIGNING_P12_PASSWORD:-}" ]]; then
  echo "::warning title=Unsigned build::MURMUR_SIGNING_P12_* secrets are not set; keeping the ad-hoc signature (permissions reset on every update)."
  exit 0
fi

APP="$(find "$ROOT/target" -path '*/release/bundle/macos/*.app' -maxdepth 6 -type d | head -1)"
DMG="$(find "$ROOT/target" -path '*/release/bundle/dmg/*.dmg' -maxdepth 6 -type f | head -1)"
[[ -d "$APP" ]] || { echo "::error::no .app bundle found"; exit 1; }
echo "app: $APP"
echo "dmg: ${DMG:-<none>}"

# A throwaway keychain holding only our identity.
WORK="$(mktemp -d)"
KEYCHAIN="$WORK/murmur-signing.keychain-db"
KC_PASS="$(openssl rand -hex 16)"
security create-keychain -p "$KC_PASS" "$KEYCHAIN"
security set-keychain-settings -lut 3600 "$KEYCHAIN"
security unlock-keychain -p "$KC_PASS" "$KEYCHAIN"
# Put it first in the search list so codesign finds the identity.
security list-keychains -d user -s "$KEYCHAIN" $(security list-keychains -d user | tr -d '"')

echo "$MURMUR_SIGNING_P12_BASE64" | base64 --decode > "$WORK/cert.p12"
security import "$WORK/cert.p12" -k "$KEYCHAIN" -P "$MURMUR_SIGNING_P12_PASSWORD" -T /usr/bin/codesign
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KC_PASS" "$KEYCHAIN" >/dev/null

# Self-signed, so trust it for code signing on this runner; codesign refuses
# untrusted identities. (Runners have passwordless sudo.)
openssl pkcs12 -in "$WORK/cert.p12" -passin env:MURMUR_SIGNING_P12_PASSWORD -nokeys -legacy 2>/dev/null \
  | openssl x509 -out "$WORK/cert.pem" \
  || openssl pkcs12 -in "$WORK/cert.p12" -passin env:MURMUR_SIGNING_P12_PASSWORD -nokeys | openssl x509 -out "$WORK/cert.pem"
sudo security add-trusted-cert -d -r trustRoot -p codeSign -k /Library/Keychains/System.keychain "$WORK/cert.pem"

IDENTITY="$(security find-identity -p codesigning "$KEYCHAIN" | awk '/^ *[0-9]+\)/ {print $2; exit}')"
[[ -n "$IDENTITY" ]] || { security find-identity -p codesigning "$KEYCHAIN"; echo "::error::signing identity not found"; exit 1; }
echo "identity: $IDENTITY"

# Sign nested code first, then the app (no --deep: it ignores entitlements).
while IFS= read -r -d '' nested; do
  codesign --force --options runtime --timestamp=none --keychain "$KEYCHAIN" --sign "$IDENTITY" "$nested"
done < <(find "$APP/Contents" \( -name '*.dylib' -o -name '*.framework' \) -print0)
codesign --force --options runtime --timestamp=none --keychain "$KEYCHAIN" \
  --entitlements "$ENTITLEMENTS" --sign "$IDENTITY" "$APP"

codesign --verify --strict --verbose=2 "$APP"
REQ="$(codesign -d -r- "$APP" 2>&1 | sed -n 's/^designated => //p')"
echo "designated requirement: $REQ"
if [[ "$REQ" != *"certificate leaf"* ]]; then
  echo "::error::designated requirement isn't tied to the certificate: $REQ"
  exit 1
fi
codesign -d --entitlements - "$APP" 2>/dev/null | grep -q "audio-input" \
  || { echo "::error::microphone entitlement missing after signing"; exit 1; }

# Rebuild the DMG around the re-signed app (drag-to-Applications layout).
if [[ -n "${DMG:-}" ]]; then
  STAGE="$WORK/dmg"
  mkdir -p "$STAGE"
  cp -R "$APP" "$STAGE/"
  ln -s /Applications "$STAGE/Applications"
  rm -f "$DMG"
  hdiutil create -volname "Murmur" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
  codesign --force --timestamp=none --keychain "$KEYCHAIN" --sign "$IDENTITY" "$DMG"
  echo "rebuilt $DMG"
fi

security delete-keychain "$KEYCHAIN" || true
echo "::notice title=Signed::Murmur signed with the stable certificate ($REQ)"
