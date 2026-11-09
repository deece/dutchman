#!/usr/bin/env python3
"""Packages one candidate build's dutchman program:

  scripts/package-candidate.py <target> <name>

Writes dist/<name>.zip (Windows) or dist/<name>.tar.gz, holding a <name> folder with the
program, LICENSE and NOTICE, and dist/<name>.<ext>.sha256 beside it. Prints the archive's name.
"""
import hashlib
import os
import sys
import tarfile
import zipfile

target, name = sys.argv[1:3]
exe = ".exe" if "windows" in target else ""
program = os.path.join("target", target, "release", "dutchman" + exe)
files = [(program, f"{name}/dutchman{exe}"), ("LICENSE", f"{name}/LICENSE"), ("NOTICE", f"{name}/NOTICE")]
os.makedirs("dist", exist_ok=True)
if exe:
    archive = f"{name}.zip"
    with zipfile.ZipFile(os.path.join("dist", archive), "w", zipfile.ZIP_DEFLATED) as z:
        for source, member in files:
            z.write(source, member)
else:
    archive = f"{name}.tar.gz"
    with tarfile.open(os.path.join("dist", archive), "w:gz", format=tarfile.USTAR_FORMAT) as t:
        for source, member in files:
            info = t.gettarinfo(source, member)
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            with open(source, "rb") as f:
                t.addfile(info, f)
with open(os.path.join("dist", archive), "rb") as f:
    digest = hashlib.sha256(f.read()).hexdigest()
with open(os.path.join("dist", archive + ".sha256"), "w") as f:
    f.write(f"{digest}  {archive}\n")
print(archive)
