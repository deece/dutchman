"""Check the candidates manifest reader and the candidate packager."""

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("candidates", ROOT / "scripts" / "candidates.py")
CANDIDATES = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CANDIDATES)

SHA = "0123456789abcdef0123456789abcdef01234567"


class Manifest(unittest.TestCase):
    def write(self, text):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        path = Path(folder.name) / "candidates.toml"
        path.write_text(text)
        return str(path)

    def test_the_committed_manifest_reads(self):
        self.assertIsInstance(CANDIDATES.load(str(ROOT / "docs" / "candidates.toml")), list)

    def test_a_candidate_gets_the_default_targets_and_repository(self):
        path = self.write(f'[[candidate]]\nname = "fix-1"\ndutchman = "{SHA}"\ncadmpeg = "{SHA}"\n')
        [candidate] = CANDIDATES.load(path)
        self.assertEqual(candidate["targets"], CANDIDATES.DEFAULT_TARGETS)
        self.assertEqual(candidate["cadmpeg_repo"], "https://github.com/cadmpeg/cadmpeg")
        self.assertEqual(candidate["note"], "fix-1")
        out = subprocess.run([sys.executable, "-I", str(ROOT / "scripts" / "candidates.py"),
                              "matrix", path], check=True, capture_output=True, text=True)
        rows = json.loads(out.stdout)["include"]
        self.assertEqual([r["target"] for r in rows], CANDIDATES.DEFAULT_TARGETS)
        self.assertEqual(rows[0]["os"], "windows-latest")

    def test_malformed_entries_are_refused(self):
        bad = [
            f'[[candidate]]\nname = "Fix 1"\ndutchman = "{SHA}"\ncadmpeg = "{SHA}"\n',
            f'[[candidate]]\nname = "fix"\ndutchman = "abc"\ncadmpeg = "{SHA}"\n',
            f'[[candidate]]\nname = "fix"\ndutchman = "{SHA}"\ncadmpeg = "{SHA}"\ntargets = ["sparc"]\n',
            f'[[candidate]]\nname = "fix"\ndutchman = "{SHA}"\ncadmpeg = "{SHA}"\n'
            f'cadmpeg_repo = "https://example.com/cadmpeg"\n',
            f'[[candidate]]\nname = "fix"\ndutchman = "{SHA}"\ncadmpeg = "{SHA}"\nbranch = "x"\n',
            f'[[candidate]]\nname = "a"\ndutchman = "{SHA}"\ncadmpeg = "{SHA}"\n' * 2,
        ]
        for text in bad:
            with self.subTest(text=text), self.assertRaises(SystemExit):
                CANDIDATES.load(self.write(text))


class Package(unittest.TestCase):
    def package(self, target):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        root = Path(folder.name)
        exe = ".exe" if "windows" in target else ""
        release = root / "target" / target / "release"
        release.mkdir(parents=True)
        (release / f"dutchman{exe}").write_bytes(b"program")
        (root / "LICENSE").write_text("licence")
        (root / "NOTICE").write_text("notice")
        out = subprocess.run([sys.executable, "-I", str(ROOT / "scripts" / "package-candidate.py"),
                              target, "dutchman-candidate-fix-1-" + target],
                             cwd=root, check=True, capture_output=True, text=True)
        archive = root / "dist" / out.stdout.strip()
        digest, name = (root / "dist" / (archive.name + ".sha256")).read_text().split()
        self.assertEqual(name, archive.name)
        self.assertEqual(digest, hashlib.sha256(archive.read_bytes()).hexdigest())
        return archive

    def test_linux_gets_a_tar_gz_with_the_program_in_its_folder(self):
        archive = self.package("x86_64-unknown-linux-gnu")
        with tarfile.open(archive) as t:
            names = t.getnames()
            self.assertIn("dutchman-candidate-fix-1-x86_64-unknown-linux-gnu/dutchman", names)

    def test_windows_gets_a_zip(self):
        archive = self.package("x86_64-pc-windows-msvc")
        with zipfile.ZipFile(archive) as z:
            self.assertIn("dutchman-candidate-fix-1-x86_64-pc-windows-msvc/dutchman.exe", z.namelist())


if __name__ == "__main__":
    unittest.main()
