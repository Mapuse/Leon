#!/usr/bin/env python3
"""Build an EFI_SIGNATURE_LIST (.esl) file from an X.509 certificate.

A .esl file is what firmware Secure Boot configuration accepts when enrolling
the PK / KEK / db keys. It is the same payload `cert-to-efi-sig-list` from
efitools produces; this implementation needs only openssl (for the PEM->DER
conversion) and the Python standard library.

Usage:
    gen-esl.py <cert.pem> <out.esl> [owner-guid]

The owner GUID defaults to all zeros; pass a UUID string to tag the signature
with a specific owner.
"""

import struct
import subprocess
import sys

# EFI_CERT_X509_GUID a5c059a1-94e4-4aa7-87b5-ab155c2bf072, stored little-endian.
EFI_CERT_X509 = struct.pack("<IHH", 0xA5C059A1, 0x94E4, 0x4AA7) + bytes.fromhex("87b5ab155c2bf072")


def pem_to_der(pem_path: str) -> bytes:
    return subprocess.run(
        ["openssl", "x509", "-in", pem_path, "-outform", "der"],
        check=True,
        capture_output=True,
    ).stdout


def guid_bytes(guid: str) -> bytes:
    if guid == "00000000-0000-0000-0000-000000000000":
        return bytes(16)
    fields = guid.split("-")
    a, b, c = (int(f, 16) for f in fields[:3])
    d = bytes.fromhex(fields[3])
    e = bytes.fromhex(fields[4])
    return struct.pack("<IHH", a, b, c) + d + e


def main() -> int:
    if len(sys.argv) not in (3, 4):
        print(__doc__, file=sys.stderr)
        return 2
    cert, out = sys.argv[1], sys.argv[2]
    owner = guid_bytes(sys.argv[3] if len(sys.argv) == 4 else "00000000-0000-0000-0000-000000000000")

    der = pem_to_der(cert)
    sig_size = 16 + len(der)
    list_size = 16 + 4 + 4 + 4 + sig_size
    esl = (
        EFI_CERT_X509
        + struct.pack("<III", list_size, 0, sig_size)
        + owner
        + der
    )
    with open(out, "wb") as f:
        f.write(esl)
    print(f"wrote {out} ({len(esl)} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
