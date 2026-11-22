"""Regenerate THIRD_PARTY_NOTICES.md, third-party/ license texts and the
package-files.json allowlist from Cargo.lock and the local cargo registry."""
import json
import pathlib
import shutil
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[1]
REGISTRY = pathlib.Path.home() / ".cargo" / "registry" / "src"
LICENSE_NAMES = ("LICENSE", "COPYING", "UNLICENSE")
RUNTIME = ["extension.json", "bitcalc.wasm", "README.md", "LICENSE", "THIRD_PARTY_NOTICES.md"]


def crate_dir(name, version):
    for index in REGISTRY.iterdir():
        candidate = index / f"{name}-{version}"
        if candidate.is_dir():
            return candidate
    raise SystemExit(f"{name} {version} is not in the cargo registry; run cargo fetch")


def main():
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
    own = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["name"]
    out_dir = ROOT / "third-party"
    shutil.rmtree(out_dir, ignore_errors=True)
    notices = ["# Third-party notices\n\nBitcalc is built from the following locked Rust dependencies. "
               "Their license texts are included in `third-party/`.\n"]
    files = list(RUNTIME)
    for pkg in sorted(lock["package"], key=lambda p: p["name"]):
        name, version = pkg["name"], pkg["version"]
        if name == own:
            continue
        if name == "vtcode-extension-api":
            source = ROOT / "vendor" / "vtcode-extension-api"
            license_id, repo = "MIT", "Vendored from the VTCode source tree (extension-api/)"
        else:
            source = crate_dir(name, version)
            meta = tomllib.loads((source / "Cargo.toml").read_text(encoding="utf-8"))["package"]
            license_id, repo = meta.get("license", "see license files"), meta.get("repository", "")
        target = out_dir / f"{name}-{version}"
        target.mkdir(parents=True)
        copied = []
        for item in sorted(source.iterdir()):
            if item.is_file() and item.name.upper().startswith(LICENSE_NAMES):
                shutil.copy2(item, target / item.name)
                copied.append(f"third-party/{name}-{version}/{item.name}")
        if not copied:
            raise SystemExit(f"No license file found for {name} {version}")
        files += copied
        notices.append(f"## {name} {version}\n\nLicense: {license_id}\n\n{repo}\n")
    (ROOT / "THIRD_PARTY_NOTICES.md").write_text("\n".join(notices), encoding="utf-8", newline="\n")
    (ROOT / "package-files.json").write_text(json.dumps(files, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(f"{len(files) - len(RUNTIME)} license files for {len(notices) - 1} crates")


if __name__ == "__main__":
    main()
