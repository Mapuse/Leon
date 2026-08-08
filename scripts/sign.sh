#!/usr/bin/env bash
# Secure Boot self-signing for lbl + the EFI-stub kernel.
#
# Generates a personal PK/KEK/db key set, produces the .esl files the firmware
# Secure Boot configuration accepts for enrollment, and signs lbl.efi and
# kernel.efi with the db key. Requires: openssl, sbsign (from sbsigntools),
# and python3 (for the .esl writer, scripts/gen-esl.py).
#
# Usage:
#   scripts/sign.sh setup [keydir]        generate keys (default keys/ under
#                                          the repo, git-ignored)
#   scripts/sign.sh sign   <in.efi> [out] [keydir]
#   scripts/sign.sh sign-all [keydir]     sign the staged ESP tree in build/esp
#                                          in place (needs `make stage` first)
#   scripts/sign.sh esl    <cert.pem>     print the .esl the cert would need
#
# After signing, enroll the generated keys in your firmware's Secure Boot
# configuration (see docs/secure-boot.md) using the .esl files. Until then the
# firmware will reject the signed images at LoadImage time.
set -euo pipefail
cd "$(dirname "$0")/.."

KEYDIR="${2:-${KEYDIR:-keys}}"
OPENSSL_OPTS=(-newkey rsa:2048 -nodes -days 3650)

have() { command -v "$1" >/dev/null 2>&1; }

for tool in openssl sbsign python3; do
    have "$tool" || { echo "error: $tool not found" >&2; exit 1; }
done

owner_guid() {
    if have uuidgen; then
        uuidgen
    else
        python3 -c "import uuid; print(uuid.uuid4())"
    fi
}

# Generates a self-signed cert/key pair plus its .esl, one role at a time.
make_key() {
    local role="$1" guid="$2"
    openssl req -x509 "${OPENSSL_OPTS[@]}" \
        -keyout "$KEYDIR/$role.key" \
        -out "$KEYDIR/$role.crt" \
        -subj "/CN=Leon Secure Boot $role/OU=Leon/"
    python3 scripts/gen-esl.py "$KEYDIR/$role.crt" "$KEYDIR/$role.esl" "$guid"
}

cmd_setup() {
    mkdir -p "$KEYDIR"
    # A single owner GUID for every signature in the set.
    local owner
    owner="$(owner_guid)"
    make_key db "$owner"
    make_key KEK "$owner"
    make_key PK "$owner"
    chmod 600 "$KEYDIR"/*.key
    echo
    echo "keys written to $KEYDIR/:"
    ls -1 "$KEYDIR"
    echo
    echo "Next:"
    echo "  1. Enroll the keys in your firmware's Secure Boot configuration"
    echo "     (see docs/secure-boot.md). Order matters: db, then KEK, then PK."
    echo "  2. make stage"
    echo "  3. scripts/sign.sh sign-all"
    echo "  4. Copy the signed build/esp files to your ESP."
}

cmd_sign() {
    local in="${1:?usage: sign.sh sign <in.efi> [out] [keydir]}"
    # Strip a trailing .efi/.EFI case-insensitively for the default name.
    local out="${2:-${in%.[Ee][Ff][Ii]}-signed.efi}"
    if [ ! -f "$KEYDIR/db.key" ] || [ ! -f "$KEYDIR/db.crt" ]; then
        echo "error: $KEYDIR/db.key and $KEYDIR/db.crt required; run 'scripts/sign.sh setup'" >&2
        exit 1
    fi
    sbsign --key "$KEYDIR/db.key" --cert "$KEYDIR/db.crt" --output "$out" "$in"
}

# Signs the loader (as \EFI\BOOT\BOOT<ARCH>.EFI) and kernel in the staged ESP
# tree in place. `make stage` must have run first.
cmd_sign_all() {
    local boot_file kernel
    case "$(uname -m)" in
        x86_64) boot_file=BOOTX64.EFI ;;
        aarch64) boot_file=BOOTAA64.EFI ;;
        *)
            echo "unsupported arch: $(uname -m) (use 'make stage ARCH=amd64' or ARCH=arm64)" >&2
            exit 1
            ;;
    esac
    for f in "build/esp/EFI/BOOT/$boot_file" build/esp/EFI/leon/kernel.efi; do
        [ -f "$f" ] || { echo "error: $f missing (run 'make stage' first)" >&2; exit 1; }
        # Sign to a temp file, then replace in place: sbsign reads and writes,
        # and we must not truncate the input before it has been read.
        local tmp="$f.tmp"
        sbsign --key "$KEYDIR/db.key" --cert "$KEYDIR/db.crt" --output "$tmp" "$f" >/dev/null
        mv -f "$tmp" "$f"
    done
    echo
    echo "Signed in place:"
    ls -l "build/esp/EFI/BOOT/$boot_file" build/esp/EFI/leon/kernel.efi
}

cmd_esl() {
    python3 scripts/gen-esl.py "${1:?usage: sign.sh esl <cert.pem>}" /dev/stdout
}

case "${1:-}" in
    setup) cmd_setup ;;
    sign) shift; cmd_sign "$@" ;;
    sign-all) cmd_sign_all ;;
    esl) shift; cmd_esl "$@" ;;
    *)
        sed -n '2,15p' "$0"
        exit 1
        ;;
esac
