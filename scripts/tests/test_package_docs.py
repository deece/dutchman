"""Installed guidance is complete, bounded, and names real Dutchman commands."""
import importlib.util
from pathlib import Path
import re
import tempfile
import unittest
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("package_docs", ROOT / "scripts/package-docs.py")
DOCS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DOCS)


class InstalledGuides(unittest.TestCase):
    def test_all_markdown_skill_resources_and_references_are_staged(self):
        with tempfile.TemporaryDirectory() as folder:
            target = Path(folder)
            files = DOCS.stage(target)
            for source in [*ROOT.glob("docs/**/*.md"), *ROOT.glob("skills/**/*")]:
                if source.is_file():
                    self.assertEqual((target / source.relative_to(ROOT)).read_bytes(), source.read_bytes())
            skill = (target / "skills/dutchman-converting/SKILL.md").read_text()
            for reference in re.findall(r"docs/[\w/-]+\.md", skill):
                self.assertTrue((target / reference).is_file(), reference)
            xml = target / "guides.wxs"
            DOCS.wix_fragment(target, files, xml)
            ns = {"w": "http://wixtoolset.org/schemas/v4/wxs"}
            doc = ET.parse(xml)
            self.assertEqual(len(doc.findall(".//w:File", ns)), len(files))
            self.assertEqual(len(doc.findall(".//w:RegistryValue[@Root='HKCU'][@KeyPath='yes']", ns)), len(files))
            directories = doc.findall(".//w:Directory", ns)
            self.assertEqual(len(doc.findall(".//w:RemoveFolder[@On='uninstall']", ns)), len(directories))

    def test_large_images_and_non_markdown_examples_are_omitted(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for directory in ("assets/install", "skills/example", "docs/images", "docs/examples"):
                (root / directory).mkdir(parents=True)
            (root / "assets/install/AGENTS.md").write_text("Instructions")
            (root / "skills/example/SKILL.md").write_text("Skill")
            (root / "skills/example/resource.txt").write_text("Skill resource")
            (root / "docs/guide.md").write_text("Guide")
            (root / "docs/images/small.png").write_bytes(b"small")
            (root / "docs/images/large.png").write_bytes(b"x" * (DOCS.IMAGE_LIMIT + 1))
            (root / "docs/examples/model.sc").write_bytes(b"example")
            target = root / "out"
            DOCS.stage(target, root)
            self.assertTrue((target / "docs/images/small.png").exists())
            self.assertTrue((target / "skills/example/resource.txt").exists())
            self.assertFalse((target / "docs/images/large.png").exists())
            self.assertFalse((target / "docs/examples/model.sc").exists())

    def test_installed_agents_names_only_existing_commands(self):
        guide = (ROOT / "assets/install/AGENTS.md").read_text()
        source = (ROOT / "crates/dutchman/src/main.rs").read_text()
        commands = re.findall(r'"\$D" ([a-z-]+)', guide)
        self.assertEqual(set(commands), {"formats", "inspect", "convert", "convert-all", "investigate"})
        for command in commands:
            variant = "".join(part.title() for part in command.split("-"))
            self.assertRegex(source, rf"(?m)^    {variant} \{{")
        self.assertIn("verification", guide)
        self.assertIn("<stem>.report.json", guide)
