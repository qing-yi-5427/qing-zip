"""Inspect PE mitigation metadata without loading or executing the image.

CFG requires both image flags and a nonempty instrumentation/function table;
the CET bit describes compatibility, not whether hardware enforcement is active.
Only --require-hardened enforces a release gate; third-party audits are read-only.
"""
import argparse
import json
import struct
from pathlib import Path


def imports(path):
    """Return ordinary and delay-loaded PE module names for a reviewed binary."""
    data=path.read_bytes()
    def u16(i):return struct.unpack_from('<H',data,i)[0]
    def u32(i):return struct.unpack_from('<I',data,i)[0]
    pe=u32(60);opt=pe+24
    directories=opt+(112 if u16(opt)==0x20b else 96)
    start=opt+u16(pe+20)
    sections=[(u32(i+12),max(u32(i+8),u32(i+16)),u32(i+20))
              for i in range(start,start+40*u16(pe+6),40)]
    def offset(addr):
        return next(raw+addr-rva for rva,size,raw in sections if rva<=addr<rva+size)
    names=set()
    for index,stride,field in [(1,20,12),(13,32,4)]:
        addr=u32(directories+8*index)
        if not addr:continue
        cursor=offset(addr)
        while u32(cursor+field):
            name=offset(u32(cursor+field))
            names.add(data[name:data.index(0,name)].decode('ascii').lower())
            cursor+=stride
    return sorted(names)

def needs_private_runtime(name):
    return name.startswith(('msvcp1', 'vcruntime1', 'concrt1', 'vcomp1')) or name == 'webview2loader.dll'


def inspect(path: Path) -> dict:
    data = path.read_bytes()

    def unpack(fmt, offset):
        return struct.unpack_from("<" + fmt, data, offset)

    if data[:2] != b"MZ":
        raise ValueError("not a PE image")
    pe = unpack("I", 0x3C)[0]
    if data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("invalid PE signature")
    machine, sections = unpack("HH", pe + 4)
    optional_size = unpack("H", pe + 20)[0]
    optional = pe + 24
    magic = unpack("H", optional)[0]
    if magic not in (0x10B, 0x20B):
        raise ValueError("unsupported optional header")
    bits = 64 if magic == 0x20B else 32
    dll_flags = unpack("H", optional + 70)[0]
    directory = optional + (112 if bits == 64 else 96)
    count = unpack("I", directory - 4)[0]
    table = optional + optional_size
    section_rows = []
    for index in range(sections):
        row = table + index * 40
        name = data[row:row + 8].rstrip(b"\0").decode("ascii", errors="replace")
        virtual_size, rva, raw_size, raw = unpack("IIII", row + 8)
        flags = unpack("I", row + 36)[0]
        section_rows.append((name, rva, max(virtual_size, raw_size), raw, raw_size, flags))

    def file_offset(rva, size=1):
        for _, start, length, raw, raw_size, _ in section_rows:
            if start <= rva < start + length and rva - start + size <= raw_size:
                return raw + rva - start
        raise ValueError(f"unmapped RVA {rva:#x}")

    def entry(index):
        return unpack("II", directory + index * 8) if index < count else (0, 0)

    guard_flags = guard_functions = 0
    config_rva, config_size = entry(10)
    if config_rva and config_size:
        config = file_offset(config_rva)
        declared = unpack("I", config)[0]
        guard_offset = 144 if bits == 64 else 88
        if min(declared, config_size) >= guard_offset + 4:
            guard_flags = unpack("I", config + guard_offset)[0]
            guard_functions = unpack("Q" if bits == 64 else "I", config + guard_offset - (8 if bits == 64 else 4))[0]
    extended_flags = 0
    debug_rva, debug_size = entry(6)
    if debug_rva:
        debug = file_offset(debug_rva, debug_size)
        for row in range(debug, debug + debug_size, 28):
            kind, size, _, raw = unpack("IIII", row + 12)
            if kind == 20 and size >= 4:
                extended_flags |= unpack("I", raw)[0]
    certificate_offset, certificate_size = entry(4)
    relocation_rva, relocation_size = entry(5)
    return {
        "path": str(path.resolve()),
        "machine": f"0x{machine:04x}",
        "bits": bits,
        "aslr": bool(dll_flags & 0x40),
        "base_relocations": bool(relocation_rva and relocation_size),
        "high_entropy_va": bool(dll_flags & 0x20),
        "dep": bool(dll_flags & 0x100),
        "cfg": bool(dll_flags & 0x4000 and guard_flags & 0x100 and guard_flags & 0x400 and guard_functions),
        "cfg_function_count": guard_functions,
        "guard_flags": f"0x{guard_flags:08x}",
        "eh_continuation_guard": bool(guard_flags & 0x400000),
        "cet_compatible": bool(extended_flags & 1),
        "certificate_present": bool(certificate_offset and certificate_size),
        "writable_executable_sections": [name for name, _, _, _, _, flags in section_rows if flags & 0x20000000 and flags & 0x80000000],
    }


def hardened_issues(result: dict, expected_machine: str | None = None) -> list[str]:
    """Gate actual input images, rather than trusting compiler flag strings."""
    required = ["aslr", "base_relocations", "dep", "cfg"]
    if result["machine"] != "0xaa64" or expected_machine != "0xaa64":
        required.append("cet_compatible")
    if result["bits"] == 64:
        required.append("high_entropy_va")
    issues = [f"missing {key}" for key in required if not result[key]]
    if result["writable_executable_sections"]:
        issues.append("writable and executable sections")
    if expected_machine is not None and result["machine"] != expected_machine.lower():
        issues.append(f"wrong machine {result['machine']}; expected {expected_machine}")
    return issues


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="+", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--require-hardened", action="store_true")
    parser.add_argument("--expect-machine", choices=["0x8664", "0x014c", "0xaa64"])
    args = parser.parse_args()
    paths = []
    for path in args.paths:
        paths.extend(sorted(item for item in path.rglob("*") if item.is_file() and item.suffix.lower() in {".exe", ".dll", ".sfx", ".pyd"}) if path.is_dir() else [path])
    results = [inspect(path) for path in paths]
    output = json.dumps(results, indent=2, ensure_ascii=False)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(output + "\n", encoding="utf-8")
    else:
        print(output)
    if args.require_hardened:
        failures = [f"{row['path']}: {', '.join(issues)}" for row in results if (issues := hardened_issues(row, args.expect_machine))]
        if not results or failures:
            raise SystemExit("PE mitigation gate failed\n" + "\n".join(failures))


if __name__ == "__main__":
    main()
