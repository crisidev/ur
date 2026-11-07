"""Checks the BTHome encryption test vector and prints example payloads for
the design doc in tmp/bthome-design.md."""

from cryptography.hazmat.primitives.ciphers.aead import AESCCM


def encrypt(key: bytes, mac_display: bytes, info: int, counter: int, plain: bytes) -> bytes:
    """Returns the service data after the UUID: info, ciphertext, counter, MIC."""
    ctr = counter.to_bytes(4, "little")
    nonce = mac_display + b"\xd2\xfc" + bytes([info]) + ctr
    sealed = AESCCM(key, tag_length=4).encrypt(nonce, plain, None)
    return bytes([info]) + sealed[:-4] + ctr + sealed[-4:]


# The published vector (bthome.io/encryption).
key = bytes.fromhex("231d39c1d7cc1ab1aee224cd096db932")
out = encrypt(key, bytes.fromhex("5448E68F80A5"), 0x41, 0x00112233, bytes.fromhex("02ca0903bf13"))
expected = bytes.fromhex("41e445f3c9962b332211006c7c4519")
print("vector ok:", out == expected, out.hex())

# Our example: on-air address bytes (LSB first, as main.rs builds them) and
# their display order, which the nonce uses.
on_air = bytes.fromhex("9a78563412e5")
display = on_air[::-1]
print("display MAC:", ":".join(f"{b:02X}" for b in display))

# Packet id 7, battery 82 %, button 2 of 4 pressed.
plain = bytes([0x00, 0x07, 0x01, 82, 0x3A, 0x00, 0x3A, 0x01, 0x3A, 0x00, 0x3A, 0x00])
flags = bytes.fromhex("020106")

svc = b"\xd2\xfc" + bytes([0x44]) + plain
adv = flags + bytes([len(svc) + 1, 0x16]) + svc
print("unencrypted adv (%d B):" % len(adv), adv.hex(" "))

svc = b"\xd2\xfc" + encrypt(key, display, 0x45, 0x00000100, plain)
adv = flags + bytes([len(svc) + 1, 0x16]) + svc
print("encrypted adv (%d B):" % len(adv), adv.hex(" "))

# With a dimmer slot: packet id 8, battery 82 %, no buttons, dimmer rotate
# right 1 step (swipe up).
plain = bytes([0x00, 0x08, 0x01, 82]) + bytes.fromhex("3a003a003a003a00") + bytes.fromhex("3c0201")
print("plaintext (%d B):" % len(plain), plain.hex(" "))
svc = b"\xd2\xfc" + bytes([0x44]) + plain
adv = flags + bytes([len(svc) + 1, 0x16]) + svc
print("unencrypted adv w/ dimmer (%d B):" % len(adv), adv.hex(" "))
svc = b"\xd2\xfc" + encrypt(key, display, 0x45, 0x00000101, plain)
adv = flags + bytes([len(svc) + 1, 0x16]) + svc
print("encrypted adv w/ dimmer (%d B):" % len(adv), adv.hex(" "))
