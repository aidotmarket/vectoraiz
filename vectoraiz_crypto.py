import os
import json
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import ed25519
from cryptography.hazmat.primitives.asymmetric import x25519
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.kdf.pbkdf2 import PBKDF2HMAC
from cryptography.hazmat.backends import default_backend
from cryptography.fernet import Fernet
import base64

KEYSTORE_PATH = "keystore.json"
PASSWORD = b"vectoraiz_secret"  # Replace with a more secure method in production

def generate_ed25519_keypair():
    private_key = ed25519.Ed25519PrivateKey.generate()
    public_key = private_key.public_key()
    return private_key, public_key

def generate_x25519_keypair():
    private_key = x25519.X25519PrivateKey.generate()
    public_key = private_key.public_key()
    return private_key, public_key

def encrypt_private_key(private_key, password):
    password = password
    salt = os.urandom(16)

    kdf = PBKDF2HMAC(
        algorithm=hashes.SHA256(),
        length=32,
        salt=salt,
        iterations=390000,
        backend=default_backend()
    )
    key = base64.urlsafe_b64encode(kdf.derive(password))
    f = Fernet(key)
    encrypted_private_key = f.encrypt(private_key.private_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PrivateFormat.Raw,
        encryption_algorithm=serialization.NoEncryption()
    ))
    return encrypted_private_key, salt

def decrypt_private_key(encrypted_private_key, salt, password, key_type):
    password = password
    kdf = PBKDF2HMAC(
        algorithm=hashes.SHA256(),
        length=32,
        salt=salt,
        iterations=390000,
        backend=default_backend()
    )
    key = base64.urlsafe_b64encode(kdf.derive(password))
    f = Fernet(key)
    decrypted_private_key = f.decrypt(encrypted_private_key)
    if key_type == "ed25519":
        return ed25519.Ed25519PrivateKey.from_private_bytes(decrypted_private_key)
    elif key_type == "x25519":
        return x25519.X25519PrivateKey.from_private_bytes(decrypted_private_key)
    else:
        raise ValueError("Invalid key type")

def save_keys(ed25519_private_key, ed25519_public_key, x25519_private_key, x25519_public_key):
    encrypted_ed25519_private_key, ed25519_salt = encrypt_private_key(ed25519_private_key, PASSWORD)
    encrypted_x25519_private_key, x25519_salt = encrypt_private_key(x25519_private_key, PASSWORD)
    keystore = {
        "ed25519_public_key": ed25519_public_key.public_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PublicFormat.Raw
        ).hex(),
        "encrypted_ed25519_private_key": encrypted_ed25519_private_key.decode('latin-1'),
        "ed25519_salt": ed25519_salt.hex(),
        "x25519_public_key": x25519_public_key.public_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PublicFormat.Raw
        ).hex(),
        "encrypted_x25519_private_key": encrypted_x25519_private_key.decode('latin-1'),
        "x25519_salt": x25519_salt.hex()
    }
    with open(KEYSTORE_PATH, "w") as f:
        json.dump(keystore, f)

def load_keys():
    try:
        with open(KEYSTORE_PATH, "r") as f:
            keystore = json.load(f)
        ed25519_public_key = ed25519.Ed25519PublicKey.from_public_bytes(bytes.fromhex(keystore["ed25519_public_key"]))
        ed25519_salt = bytes.fromhex(keystore["ed25519_salt"])
        encrypted_ed25519_private_key = keystore["encrypted_ed25519_private_key"].encode('latin-1')
        ed25519_private_key = decrypt_private_key(encrypted_ed25519_private_key, ed25519_salt, PASSWORD, "ed25519")

        x25519_public_key = x25519.X25519PublicKey.from_public_bytes(bytes.fromhex(keystore["x25519_public_key"]))
        x25519_salt = bytes.fromhex(keystore["x25519_salt"])
        encrypted_x25519_private_key = keystore["encrypted_x25519_private_key"].encode('latin-1')
        x25519_private_key = decrypt_private_key(encrypted_x25519_private_key, x25519_salt, PASSWORD, "x25519")

        return ed25519_private_key, ed25519_public_key, x25519_private_key, x25519_public_key
    except FileNotFoundError:
        return None, None, None, None

def get_or_create_keypair():
    ed25519_private_key, ed25519_public_key, x25519_private_key, x25519_public_key = load_keys()
    if not ed25519_private_key or not ed25519_public_key or not x25519_private_key or not x25519_public_key:
        ed25519_private_key, ed25519_public_key = generate_ed25519_keypair()
        x25519_private_key, x25519_public_key = generate_x25519_keypair()
        save_keys(ed25519_private_key, ed25519_public_key, x25519_private_key, x25519_public_key)
    return ed25519_private_key, ed25519_public_key, x25519_private_key, x25519_public_key

if __name__ == "__main__":
    ed25519_private_key, ed25519_public_key, x25519_private_key, x25519_public_key = get_or_create_keypair()
    print("Ed25519 Private Key:", ed25519_private_key)
    print("Ed25519 Public Key:", ed25519_public_key)
    print("X25519 Private Key:", x25519_private_key)
    print("X25519 Public Key:", x25519_public_key)

    # Test loading the keys
    loaded_ed25519_private_key, loaded_ed25519_public_key, loaded_x25519_private_key, loaded_x25519_public_key = load_keys()
    assert loaded_ed25519_private_key is not None
    assert loaded_ed25519_public_key is not None
    assert loaded_x25519_private_key is not None
    assert loaded_x25519_public_key is not None
    print("Key loading test passed!")
