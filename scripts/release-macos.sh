#!/bin/bash
# Called after compilation; credentials exist only for this signing step.
set +x
set -euo pipefail
umask 077

fail() { echo "::error::$*" >&2; exit 1; }

[[ "$(uname -s)" == Darwin ]] || fail 'macOS signing requires a Mac.'
[[ $# -eq 2 ]] || fail 'Usage: release-macos.sh <target> <arch>'
target=$1
arch=$2
script_dir=$(cd "$(dirname "$0")" && pwd)
case "$target/$arch" in
  x86_64-apple-darwin/x64|aarch64-apple-darwin/arm64) ;;
  *) fail 'Unsupported macOS release target.' ;;
esac
checkpoint="notarization-pending/$arch"
[[ ! -e "$checkpoint/manifest.json" ]] || fail 'A submitted checkpoint already exists; use the notarization resume workflow.'
for name in APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_API_KEY APPLE_API_PRIVATE_KEY RUNNER_TEMP; do
  [[ -n "${!name:-}" ]] || fail "Missing signing variable: $name"
done
[[ "$APPLE_SIGNING_IDENTITY" == 'Developer ID Application: '* ]] || fail 'A Developer ID Application identity is required.'

signing_dir=$(mktemp -d "$RUNNER_TEMP/luna-macos-signing.XXXXXX")
keychain="$signing_dir/signing.keychain-db"
original_keychains=()
while IFS= read -r item; do
  original_keychains+=("$item")
done < <(security list-keychains -d user | sed 's/^[[:space:]]*"//; s/"[[:space:]]*$//')
cleanup() {
  security list-keychains -d user -s "${original_keychains[@]}" >/dev/null 2>&1 || true
  security delete-keychain "$keychain" >/dev/null 2>&1 || true
  rm -rf "$signing_dir"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

keychain_password=$(openssl rand -hex 32)
printf '%s' "$APPLE_CERTIFICATE" | base64 --decode > "$signing_dir/certificate.p12"
printf '%s' "$APPLE_API_PRIVATE_KEY" > "$signing_dir/notary.p8"
security create-keychain -p "$keychain_password" "$keychain" >/dev/null
security set-keychain-settings -lut 7200 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
security import "$signing_dir/certificate.p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" >/dev/null
security list-keychains -d user -s "$keychain" "${original_keychains[@]}"
security find-identity -v -p codesigning "$keychain" | grep -F -- "\"$APPLE_SIGNING_IDENTITY\"" >/dev/null || fail 'Signing identity was not imported.'
rm "$signing_dir/certificate.p12"

# Prevent Tauri from importing another copy or using its team-key-only notarizer.
unset APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_API_PRIVATE_KEY keychain_password
unset APPLE_API_ISSUER APPLE_API_KEY_PATH APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID
export APPLE_SIGNING_IDENTITY
# Request both outputs: a DMG-only bundle deletes its intermediate .app.
npm exec tauri bundle -- --bundles app,dmg --target "$target"

shopt -s nullglob
apps=(app/native/target/"$target"/release/bundle/macos/*.app)
dmgs=(app/native/target/"$target"/release/bundle/dmg/*.dmg)
[[ ${#apps[@]} -eq 1 && ${#dmgs[@]} -eq 1 ]] || fail 'Expected exactly one app and one DMG.'
app=${apps[0]}
dmg=${dmgs[0]}
codesign --verify --deep --strict "$app"
signature_details=$(codesign -dvv "$app" 2>&1)
grep -Fx -- "Authority=$APPLE_SIGNING_IDENTITY" <<< "$signature_details" >/dev/null || fail 'App has an unexpected signing identity.'
grep -E 'flags=.*runtime' <<< "$signature_details" >/dev/null || fail 'App is missing hardened runtime.'
# Check the signed helper without starting a browser.
"$app/Contents/MacOS/agent-browser" --version
codesign --force --sign "$APPLE_SIGNING_IDENTITY" --timestamp "$dmg"
codesign --verify --strict "$dmg"
hdiutil verify "$dmg"

# Persist exact signed bytes before waiting; this directory contains no keys.
mkdir -p "$checkpoint"
cp "$dmg" "$checkpoint/Luna-Mux_macos-$arch.dmg"
ditto -c -k --keepParent "$app" "$checkpoint/app.zip"
# Individual API keys must omit --issuer. Submit exactly once, without --wait.
xcrun notarytool submit "$checkpoint/Luna-Mux_macos-$arch.dmg" --key "$signing_dir/notary.p8" --key-id "$APPLE_API_KEY" \
  --output-format json > "$signing_dir/notary-result.json"
submission_id=$(python3 "$script_dir/macos-notarization-checkpoint.py" create "$checkpoint" "$target" "$arch" "$signing_dir/notary-result.json" "$(basename "$app")")
echo "Apple notarization submission: $submission_id"
echo 'Upload the signed checkpoint before waiting for Apple. It contains no signing credentials.'
