#!/usr/bin/env python3
"""Create a deterministic gzip-compressed tar archive on Linux or macOS."""

from __future__ import annotations

import gzip
import pathlib
import sys
import tarfile


def normalized(info: tarfile.TarInfo) -> tarfile.TarInfo:
    info.uid = 0
    info.gid = 0
    info.uname = ""
    info.gname = ""
    info.mtime = 0
    return info


def main() -> int:
    if len(sys.argv) != 4:
        print("usage: create-release-archive.py <root> <name> <archive>", file=sys.stderr)
        return 2

    root = pathlib.Path(sys.argv[1]).resolve()
    name = sys.argv[2]
    source = root / name
    archive = pathlib.Path(sys.argv[3])
    if not source.is_dir() or source.parent != root:
        print(f"release stage is missing or unsafe: {source}", file=sys.stderr)
        return 1

    paths = [source, *sorted(source.rglob("*"), key=lambda path: path.as_posix())]
    with archive.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as output:
                for path in paths:
                    arcname = path.relative_to(root).as_posix()
                    info = normalized(output.gettarinfo(path, arcname))
                    if info.isfile():
                        with path.open("rb") as content:
                            output.addfile(info, content)
                    else:
                        output.addfile(info)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
