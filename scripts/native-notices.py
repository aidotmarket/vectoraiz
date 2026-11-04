"""Append native library notices to the generated Rust and UI notice artifact."""
from pathlib import Path

root = Path(__file__).resolve().parents[1]
print('\nModel Context Protocol schemas (dual-era bridge validation):')
print((root / 'crates/vz-mcp/schema/PIN.md').read_text())
print((root / 'crates/vz-mcp/schema/LICENSE').read_text())
print('\nllama.cpp 244641955f6146f7e8474afff7772d427593a534\n')
print((root / 'docs/phase0/llama-cpp-LICENSE.txt').read_text())
pdfium = root / '.assets/pdfium'
for path in [pdfium / 'LICENSE', *sorted((pdfium / 'licenses').iterdir())]:
    if path.is_file():
        print(f'\nPDFium chromium/8111: {path.relative_to(pdfium)}\n')
        data = path.read_bytes()
        try: text = data.decode('utf-8')
        except UnicodeDecodeError: text = data.decode('latin-1')
        print(text)
print('\nProvisional model licences and source notices:')
print((root / 'spike/fixtures/models.json').read_text())

print('\nInstalled-app test model licences and source notices:')
print((root / 'spike/fixtures/models.tiers.json').read_text())

print('\nRelease model licences and source notices:')
print((root / 'spike/fixtures/models.release.json').read_text())
for path in sorted((root / 'docs/phase0/model-licenses').glob('*.txt')):
    print(f'\n{path.name}\n')
    print(path.read_text())

print('\nPinned model source/distributor NOTICE inventory:')
print((root / 'docs/phase0/model-licenses/inventory.json').read_text())

print('\ntauri-plugin-opener 2.7.2: MIT OR Apache-2.0; Rust dependency licence text is included by cargo-about above.\nhttps://github.com/tauri-apps/plugins-workspace/tree/opener-v2.7.2/plugins/opener')

print("\ncore-foundation-sys 0.8.7: MIT OR Apache-2.0; macOS system date formatter bindings; Rust dependency licence text is included by cargo-about above.\nhttps://github.com/servo/core-foundation-rs")
