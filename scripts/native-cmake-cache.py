"""Let pinned llama.cpp regenerate native projects after Rust cache restoration.

Rust caches can retain CMakeCache.txt while excluding generated Makefiles. The
pinned build script uses always_configure(false), so that combination cannot
repair itself. Keep compiled objects, but require configuration when it builds.
"""
from pathlib import Path

for cache in Path("target").glob("**/build/llama-cpp-sys-2-*/out/build/CMakeCache.txt"):
    cache.unlink()
    print(f"Removed restored CMake configuration: {cache}")
