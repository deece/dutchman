"""Synthetic Rust/diff and release fixtures; no network or Rust build required."""
from collections import Counter
from datetime import datetime, timedelta, timezone
import difflib
import importlib.util
import json
import subprocess
import tempfile
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location('gates', Path(__file__).parents[1] / 'check-version-gates.py')
gates = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gates)
FILE = 'crates/dutchman-solidcraft/src/example.rs'
OLD = 'if target >= SolidCraftVersion::V0_22_1 { flatten(); }\n'
NOW = datetime(2026, 11, 11, tzinfo=timezone.utc)


class VersionGates(unittest.TestCase):
    def audit_diff(self, before, addition, allowed_addition=False):
        # Fixture unified diff, applied to its synthetic base as a PR would be.
        diff = list(difflib.unified_diff(before.splitlines(True),
                                        (before + addition).splitlines(True)))
        self.assertTrue(any(line.startswith('+') for line in diff[2:]))
        after = ''.join(line[1:] for line in diff[2:] if line.startswith((' ', '+')))
        base = gates.inventory({FILE: before})
        current = gates.inventory({FILE: after})
        allowed = Counter(row[0] for row in base)
        new_allowed = Counter(row[0] for row in current) if allowed_addition else allowed
        return gates.check(current, base, new_allowed, allowed, (0, 30, 0))

    def test_fixture_diff_adding_old_gate_fails_with_location_and_rule(self):
        fixture = (Path(__file__).parent / 'fixtures/version-gate-old.diff').read_text()
        added = ''.join(line[1:] for line in fixture.splitlines(True)
                        if line.startswith('+') and not line.startswith('+++'))
        errors = self.audit_diff('', added)
        self.assertEqual(len(errors), 1)
        self.assertIn(FILE + ':1:', errors[0])
        self.assertIn('V0_22_1', errors[0])
        self.assertIn(gates.RULE, errors[0])

    def test_gate_at_window_passes(self):
        self.assertEqual(self.audit_diff('', OLD.replace('22_1', '30_0')), [])

    def test_allowed_old_gate_passes(self):
        self.assertEqual(self.audit_diff(OLD, '// documentation\n'), [])

    def test_allowlist_cannot_grow_even_for_current_gate(self):
        errors = self.audit_diff('', OLD.replace('22_1', '30_0'), True)
        self.assertTrue(any('only shrinks' in e for e in errors))

    def test_duplicate_old_gate_is_not_grandfathered(self):
        self.assertTrue(self.audit_diff(OLD, OLD))

    def test_version_forms_and_multiline(self):
        for statement in [
            'const MIN_VERSION: [u64; 3] = [0,\n 22, 1];',
            'const MIN: SolidCraftVersion = SolidCraftVersion([0, 22, 1]);',
            'if target < "0.22.1".parse()? { flatten(); }',
            'if target.minor() < "0.22" { flatten(); }',
            'if target >= [0, 22, 1] { flatten(); }',
            'match feature { Self::New => SolidCraftVersion::V0_22_1, }',
            'if target >= SolidCraftVersion::V0_22_1\n { flatten(); }',
        ]:
            with self.subTest(statement=statement):
                self.assertTrue(self.audit_diff('', statement + '\n'))

    def test_new_use_of_old_capability_or_constant_fails(self):
        for before, addition in [
            ('impl VersionGate { fn minimum(self) { match self { Self::Cone => SolidCraftVersion::V0_24_1, } } }\n',
             'if VersionGate::Cone.supported(target) { new_feature(); }\n'),
            ('const MIN_VERSION: [u64; 3] = [0, 22, 1];\n',
             'if target >= MIN_VERSION { new_feature(); }\n'),
            ('const MIN_VERSION: [u64; 3] = [0, 22, 1];\nconst ALIAS: [u64; 3] = MIN_VERSION;\n',
             'if target >= ALIAS { new_feature(); }\n'),
        ]:
            self.assertTrue(self.audit_diff(before, addition))

    def test_comments_test_modules_and_raw_strings_do_not_create_gates(self):
        source = '''// SolidCraftVersion::V0_22_1
/* outer /* SolidCraftVersion::V0_22_1 */ nested */
#[cfg(test)] mod tests { fn test() { let x = SolidCraftVersion::V0_22_1; } }
let text = r#"SolidCraftVersion::V0_22_1 { //"#;
'''
        # Raw strings must not be scanned for embedded Rust names.
        self.assertEqual(gates.inventory({FILE: source}), [])

    def test_numeric_string_defaults_are_not_gates(self):
        source = 'let row = json!({"name": name, "k_factor": "0.44", "ratio": "0.5", "scale": "1.0"});\n'
        self.assertEqual(gates.inventory({FILE: source}), [])
        self.assertFalse(self.audit_diff('', source))

    def test_production_cfg_is_scanned(self):
        for cfg in ('not(test)', 'any(test, unix)'):
            self.assertTrue(self.audit_diff('', f'#[cfg({cfg})] fn feature() {{ {OLD} }}'))

    def test_adding_current_table_arm_does_not_change_old_gate(self):
        before = 'match self { Self::Old => SolidCraftVersion::V0_22_1, }'
        after = before.replace(', }', ', Self::New => SolidCraftVersion::V0_30_0, }')
        base = gates.inventory({FILE: before})
        self.assertEqual(gates.check(gates.inventory({FILE: after}), base,
                                    Counter(), Counter(), (0, 30, 0)), [])

    def test_formatting_and_comments_do_not_change_gate_identity(self):
        base = gates.inventory({FILE: OLD})
        after = gates.inventory({FILE: OLD.replace('target >=', 'target /* reason */\n >=')})
        self.assertEqual([r[0] for r in base], [r[0] for r in after])

    def test_bootstrap_only_allows_actual_base_inventory(self):
        base = gates.inventory({FILE: OLD})
        allowed = Counter(row[0] for row in base)
        self.assertEqual(gates.check(base, base, allowed, Counter(), (0, 30, 0), True), [])
        allowed['invented gate'] = 1
        self.assertTrue(gates.check(base, base, allowed, Counter(), (0, 30, 0), True))


class ReleaseWindow(unittest.TestCase):
    def row(self, version, age, **extra):
        return dict(tag_name='v' + version, published_at=(NOW - age).isoformat(), **extra)

    def test_boundary_and_newest_are_included(self):
        rows = [self.row('0.18.2', timedelta(days=30)),
                self.row('0.29.1', timedelta(days=14, seconds=1)),
                self.row('0.29.2', timedelta(days=14)),
                self.row('0.30.0', timedelta(days=1)),
                self.row('0.31.0-rc.1', timedelta(), prerelease=True),
                self.row('0.32.0', timedelta(), draft=True),
                self.row('0.33.0', timedelta(days=-1))]
        newest, oldest, _ = gates.policy.window(rows, NOW)
        self.assertEqual((newest, oldest), ((0, 30, 0), (0, 29, 2)))

    def test_plain_tag_counts_even_when_flagged_prerelease(self):
        # SolidCraft can publish a final X.Y.Z with the pre-release flag still set.
        newest, _, _ = gates.policy.window([
            self.row('0.30.1', timedelta(days=9)),
            self.row('0.40.0', timedelta(hours=1), prerelease=True)], NOW)
        self.assertEqual(newest, (0, 40, 0))

    def test_newest_remains_when_no_release_in_fourteen_days(self):
        newest, oldest, _ = gates.policy.window([
            self.row('0.30.0', timedelta(days=40)),
            self.row('0.29.0', timedelta(days=50))], NOW)
        self.assertEqual(newest, oldest)

    def test_print_and_repository_check_with_saved_metadata(self):
        scripts = Path(__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory() as folder:
            metadata = Path(folder) / 'releases.json'
            metadata.write_text(json.dumps([
                dict(tag_name='v0.30.0', published_at='2020-01-01T00:00:00Z'),
                dict(tag_name='v0.18.2', published_at='2019-01-01T00:00:00Z')]))
            printed = subprocess.run(['bash', str(scripts / 'solidcraft-window.sh'),
                                      '--print', '--releases', str(metadata)],
                                     text=True, capture_output=True)
            self.assertEqual(printed.returncode, 0, printed.stderr)
            self.assertIn('Newest: 0.30.0 (published', printed.stdout)
            self.assertIn('Oldest in window: 0.30.0 (published', printed.stdout)
            self.assertIn('Opens-in floor: 0.18.2', printed.stdout)

    def test_missing_metadata_fails_closed(self):
        for rows in ([], [dict(tag_name='v0.30.0')]):
            with self.assertRaises((ValueError, KeyError)):
                gates.policy.window(rows, NOW)


if __name__ == '__main__':
    unittest.main()
