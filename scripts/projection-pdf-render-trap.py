#!/usr/bin/env python3
"""Fail projection tests if PDFium bitmap/render APIs are called (macOS/Windows).
Builds a forwarding library in private scratch and verifies the trap first.
"""
import argparse
import ctypes
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys


def run(argv, **kwargs):
    proxy = ["rtk", "run"] if shutil.which("rtk") else []
    return subprocess.run(proxy + list(map(str, argv)), check=True, **kwargs)


def pe_exports(path):
    data = path.read_bytes()
    pe = struct.unpack_from("<I", data, 0x3c)[0]
    sections, optional_size = struct.unpack_from("<H", data, pe + 6)[0], struct.unpack_from("<H", data, pe + 20)[0]
    optional = pe + 24
    directory = optional + (112 if struct.unpack_from("<H", data, optional)[0] == 0x20b else 96)
    table = optional + optional_size
    def offset(rva):
        for i in range(sections):
            size, address, raw_size, raw = struct.unpack_from("<IIII", data, table + i * 40 + 8)
            if address <= rva < address + max(size, raw_size):
                return raw + rva - address
        raise ValueError("export RVA outside PE sections")
    exports = offset(struct.unpack_from("<I", data, directory)[0])
    count = struct.unpack_from("<I", data, exports + 24)[0]
    names = struct.unpack_from("<I", data, exports + 32)[0]
    names = offset(names)
    result = []
    for i in range(count):
        start = offset(struct.unpack_from("<I", data, names + i * 4)[0])
        result.append(data[start:data.index(b"\0", start)].decode("ascii"))
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--library", type=Path, required=True)
    parser.add_argument("--scratch", type=Path, required=True)
    args = parser.parse_args()
    args.scratch.mkdir(parents=True, exist_ok=True)
    library = args.library.resolve()
    windows = sys.platform == "win32"
    if not windows and sys.platform != "darwin":
        parser.error("render trap currently supports macOS and Windows")
    if windows:
        real = args.scratch / "pdfium_real.dll"
        shutil.copyfile(library, real)
        exports = pe_exports(real)
    else:
        exports = [line.split()[-1].removeprefix("_") for line in run(["nm", "-gU", library], capture_output=True, text=True).stdout.splitlines() if line.split()]
    traps = [name for name in exports if name.startswith("FPDF_Render") or name.startswith("FPDFBitmap_Create") or name in {"FPDFImageObj_GetBitmap", "FPDFImageObj_GetRenderedBitmap", "FPDFTextObj_GetRenderedBitmap"}]
    assert "FPDF_RenderPageBitmap" in traps
    source = args.scratch / "trap.c"
    export = "__declspec(dllexport) " if windows else '__attribute__((visibility("default"))) '
    code = "#include <stdlib.h>\n" + "\n".join(export + "void " + name + "(void) { _Exit(86); }" for name in traps)
    if windows:
        # Put forwarders directly in the object file's linker directives. The
        # shim defines only traps; other exports belong to the renamed DLL.
        # PRIVATE keeps these runtime-only exports out of the import library.
        code += "\n" + "\n".join(
            f'#pragma comment(linker, "/export:{name}=pdfium_real.{name},PRIVATE")'
            for name in exports if name not in traps
        )
    source.write_text(code + "\n")
    shim = args.scratch / ("pdfium.dll" if windows else "libpdfium-trap.dylib")
    if windows:
        run(["cl", "/nologo", "/LD", source, "/link", "/OUT:" + str(shim)], cwd=args.scratch)
    else:
        run(["clang", "-dynamiclib", "-Wl,-headerpad_max_install_names", source, "-o", shim, "-Wl,-reexport_library," + str(library)])
        run(["install_name_tool", "-change", "./libpdfium.dylib", library, shim])
        run(["codesign", "--force", "--sign", "-", shim])
    # Python 3.8+ does not use PATH to resolve dependencies of ctypes DLLs.
    # Keep the directory handle alive while loading/calling Windows forwarders.
    child = """import ctypes, os, sys
from pathlib import Path
directory = os.add_dll_directory(str(Path(sys.argv[1]).parent)) if sys.platform == "win32" else None
pdfium = ctypes.CDLL(sys.argv[1])
pdfium.FPDF_InitLibrary()
pdfium.FPDF_DestroyLibrary()
print("render trap: non-render forwarding control passed", flush=True)
getattr(pdfium, sys.argv[2])()
"""
    proxy = ["rtk", "run"] if shutil.which("rtk") else []
    env = dict(os.environ, VZ_TEST_PDFIUM=str(shim.resolve()), PATH=str(args.scratch.resolve()) + os.pathsep + os.environ["PATH"])
    for name in traps:
        positive = subprocess.run(proxy + [sys.executable, "-c", child, str(shim.resolve()), name], env=env)
        assert positive.returncode == 86, f"render trap positive control failed for {name}: {positive.returncode}"
        print(f"render trap: {name} positive control exited 86", flush=True)
    run(["cargo", "test", "-p", "vz-core", "--lib", "office::projection::v2_tests::projection_pdf_", "--", "--nocapture"], env=env)
    print(f"render trap: {len(traps)} APIs trapped; positive control and PDF projection regressions passed")


if __name__ == "__main__":
    main()
