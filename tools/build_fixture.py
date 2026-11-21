"""构建验收夹具 mod：tests/fixture-energy 里的 *.in 复制到 target/fixture-energy 再 cargo horizon-mod build。

夹具放在 `target/` 里构建，是因为主 mod 打包时会扫描整个项目目录，发现嵌套的
Cargo.toml / horizon-mod.toml 会报 "unexpected package path"；所以源码用 .in 后缀存放。
用法：python -I tools/build_fixture.py        （accept.py 缺夹具时也会自动调用）
"""
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "tests" / "fixture-energy"
OUT = ROOT / "target" / "fixture-energy"
HMOD = OUT / "target" / "horizon-mod" / "jadefixture.hmod"


def build():
    (OUT / "src").mkdir(parents=True, exist_ok=True)
    sdk = (ROOT / ".horizon").as_posix()
    (OUT / "Cargo.toml").write_text((SRC / "Cargo.toml.in").read_text(encoding="utf-8").replace("@SDK@", sdk), encoding="utf-8")
    shutil.copy2(SRC / "horizon-mod.toml.in", OUT / "horizon-mod.toml")
    shutil.copy2(SRC / "lib.rs.in", OUT / "src" / "lib.rs")
    code = subprocess.run(["cargo", "horizon-mod", "build", "--wasm"], cwd=OUT).returncode
    if code != 0 or not HMOD.exists():
        sys.exit("夹具 mod 构建失败")
    return HMOD


if __name__ == "__main__":
    print(build())
