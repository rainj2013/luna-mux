#!/bin/bash
# Resume exactly the saved submission. Never compile, sign, or submit again.
set +x
set -euo pipefail
umask 077
fail() { echo "::error::$*" >&2; exit 1; }
[[ "$(uname -s)" == Darwin && $# -eq 2 ]] || fail 'Usage: finalize-macos-notarization.sh <target> <arch> on macOS'
target=$1
arch=$2
script_dir=$(cd "$(dirname "$0")" && pwd)
checkpoint="notarization-pending/$arch"
submission_id=$(python3 "$script_dir/macos-notarization-checkpoint.py" validate "$checkpoint" "$target" "$arch")
for name in APPLE_API_KEY APPLE_API_PRIVATE_KEY APPLE_SIGNING_IDENTITY RUNNER_TEMP; do
  [[ -n "${!name:-}" ]] || fail "Missing notarization variable: $name"
done
work=$(mktemp -d "$RUNNER_TEMP/luna-macos-notary.XXXXXX")
trap 'rm -rf "$work"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
printf '%s' "$APPLE_API_PRIVATE_KEY" > "$work/notary.p8"
unset APPLE_API_PRIVATE_KEY APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD
echo "Apple notarization submission: $submission_id"
echo 'The original signed checkpoint is saved as an Actions artifact; waiting never resubmits it.'
# Normal output makes Apple's waiting progress visible in the Actions log.
xcrun notarytool wait "$submission_id" --key "$work/notary.p8" --key-id "$APPLE_API_KEY" \
  --timeout "${NOTARY_WAIT_TIMEOUT:-30m}" || true
xcrun notarytool info "$submission_id" --key "$work/notary.p8" --key-id "$APPLE_API_KEY" \
  --output-format json > "$work/result.json"
python3 - "$work/result.json" "$submission_id" <<'PY'
import json, sys
with open(sys.argv[1]) as stream:
    result = json.load(stream)
if result.get('id') != sys.argv[2]:
    raise SystemExit('Apple returned a different submission')
status = result.get('status', 'unknown')
print(f'Apple notarization status: {status}', flush=True)
if status == 'In Progress':
    print('::warning::Apple is still processing. Run Resume macOS notarization with the original run ID after approval. No rebuild or resubmission is needed.')
    raise SystemExit(75)
if status != 'Accepted':
    raise SystemExit('Apple did not accept this submission; no release artifact will be uploaded.')
PY
rm "$work/notary.p8"
app_name=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["appName"])' "$checkpoint/manifest.json")
ditto -x -k "$checkpoint/app.zip" "$work/application"
app="$work/application/$app_name"
dmg="$work/Luna-Mux_macos-$arch.dmg"
# Only mutate working copies. Failed stapling/verification can resume again.
cp "$checkpoint/Luna-Mux_macos-$arch.dmg" "$dmg"
codesign --verify --deep --strict "$app"
signature_details=$(codesign -dvv "$app" 2>&1)
grep -Fx -- "Authority=$APPLE_SIGNING_IDENTITY" <<< "$signature_details" >/dev/null || fail 'Restored app has an unexpected signing identity.'
grep -E 'flags=.*runtime' <<< "$signature_details" >/dev/null || fail 'Restored app is missing hardened runtime.'
codesign --verify --strict "$dmg"
dmg_signature=$(codesign -dvv "$dmg" 2>&1)
grep -Fx -- "Authority=$APPLE_SIGNING_IDENTITY" <<< "$dmg_signature" >/dev/null || fail 'Restored DMG has an unexpected signing identity.'
"$app/Contents/MacOS/agent-browser" --version
xcrun stapler staple "$app"
xcrun stapler validate "$app"
xcrun stapler staple "$dmg"
xcrun stapler validate "$dmg"
spctl --assess --type execute --verbose=2 "$app"
spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
hdiutil verify "$dmg"
mkdir -p release-assets
cp "$dmg" "release-assets/Luna-Mux_macos-$arch.dmg"
