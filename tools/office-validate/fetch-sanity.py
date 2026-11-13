"""Download public POI originals; never substitute these for delivered outputs."""
import hashlib
from pathlib import Path
import sys
from urllib.request import urlopen

COMMIT = "77df76b9c990cadbb3c8ae72f05a03fa06194c7a"
FIXTURES = [
    ("document/Bug66263-paragraph.docx", "41ccacadeda1ac97485b772404228b9a66c2339d46208271b932b7a4a6661998"),
    ("spreadsheet/SimpleMultiCell.xlsx", "119b6e70c5bbca6ca3e46f981e0719c154d87203adb4b5b61210d726deab9f7a"),
]


def main():
    target = Path(sys.argv[1])
    target.mkdir(parents=True, exist_ok=True)
    for source, digest in FIXTURES:
        with urlopen(f"https://raw.githubusercontent.com/apache/poi/{COMMIT}/test-data/{source}", timeout=60) as response:
            data = response.read()
        if hashlib.sha256(data).hexdigest() != digest:
            raise ValueError(f"SHA256 mismatch: {source}")
        destination = target / Path(source).name
        if destination.exists():
            raise FileExistsError(destination)
        destination.write_bytes(data)
        print(f"Verified POI original: {source} sha256={digest}")


if __name__ == "__main__":
    main()
