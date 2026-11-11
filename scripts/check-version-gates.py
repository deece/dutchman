#!/usr/bin/env python3
"""Reject new SolidCraft gates below the live release window; never grow the allowlist.

Scan production Rust files, excluding comments and test-only items. Deliberately conservative:
version-bearing statements are gates even if they only declare a constant or table row.
Keys include the file and whitespace-normalized statement, with occurrence counts, so moving
lines is harmless but duplicating an old gate or using it for another feature is checked.
The base is mandatory in CI; local runs default to HEAD and include unstaged edits.
"""
import argparse
from bisect import bisect_right
from collections import Counter
import importlib.util
from pathlib import Path
import re
import subprocess
import sys

SPEC = importlib.util.spec_from_file_location("version_policy", Path(__file__).with_name("solidcraft-version-policy.py"))
policy = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(policy)
TREE = "crates/dutchman-solidcraft"
ALLOW = "scripts/version-gates-allowed.txt"
RULE = ('New work targets only the newest SolidCraft release plus releases published in the last '
        '14 days. Never back-port a new feature or add a version gate older than the window; '
        'flatten it with a named reason so the file still opens.')
# Rust strings (including raw strings) are single tokens: braces/comments inside them are inert.
LEX = re.compile(r'//[^\n]*|/\*|r(?P<hash>\#*)".*?"(?P=hash)|"(?:\\.|[^"\\])*"|'
                 r"\b\w+\b|::|=>|>=|<=|==|!=|[^\s]", re.S)
NAMED = re.compile(r'\bV(\d+)_(\d+)_(\d+)\b')
ARRAY = re.compile(r'\[\s*(\d+)\s*,\s*(\d+)\s*(?:,\s*(\d+)\s*,?)?\s*\]')
STRING = re.compile(r'"v?(\d+)\.(\d+)(?:\.(\d+))?"')


def tokens(source):
    result = []
    newlines = [m.start() for m in re.finditer("\n", source)]
    iterator = LEX.finditer(source)
    comment_end = 0
    for match in iterator:
        if match.start() < comment_end:
            continue
        token = match.group()
        if token == "/*":
            depth, pos = 1, match.end()
            while depth:
                marker = re.search(r'/\*|\*/', source[pos:])
                if marker is None:
                    raise ValueError("Unclosed Rust block comment")
                depth += 1 if marker.group() == "/*" else -1
                pos += marker.end()
            comment_end = pos
        elif not token.startswith("//"):
            result.append((token, bisect_right(newlines, match.start()) + 1))
    # Omit cfg(test) items, balancing tokenized braces rather than slicing at the first test.
    out, i = [], 0
    while i < len(result):
        attribute_end = i + 2
        if [x[0] for x in result[i:i + 2]] == ['#', '[']:
            while attribute_end < len(result) and result[attribute_end][0] != ']':
                attribute_end += 1
        attribute = [x[0] for x in result[i:attribute_end + 1]]
        if attribute in (['#', '[', 'cfg', '(', 'test', ')', ']'],
                         ['#', '[', 'test', ']']):
            i = attribute_end + 1
            while i < len(result) and result[i][0] not in ('{', ';'):
                i += 1
            if i < len(result) and result[i][0] == '{':
                depth = 1
                i += 1
                while i < len(result) and depth:
                    depth += (result[i][0] == '{') - (result[i][0] == '}')
                    i += 1
            else:
                i += 1
        else:
            out.append(result[i])
            i += 1
    return out


def inventory(files):
    """Return (stable key, version, file, line) for each version-bearing statement."""
    parsed = {name: tokens(source) for name, source in files.items()}
    symbols = {}
    # Resolve constants, including non-Vx_y_z spellings, and named capability table arms.
    all_text = ' '.join(' '.join(t for t, _ in ts) for ts in parsed.values())
    for name, expression in re.findall(r'\bconst (\w+)\s*:[^=]+ = ([^;]+);', all_text):
        matches = list(NAMED.finditer(expression)) + list(ARRAY.finditer(expression)) + list(STRING.finditer(expression))
        if matches and ("version" in name.lower() or "SOLIDCRAFT" in name or "WINDOW" in name or name.startswith('V') or 'Self' in expression):
            symbols[name] = tuple(int(n or 0) for n in matches[-1].groups())
    # Follow aliases too, so a new use cannot hide an old release behind another name.
    constants = re.findall(r'\bconst (\w+)\s*:[^=]+ = ([^;]+);', all_text)
    for _ in range(len(constants)):
        changed = False
        for name, expression in constants:
            references = [symbols[word] for word in re.findall(r'\b\w+\b', expression) if word in symbols]
            if name not in symbols and references:
                symbols[name] = min(references)
                changed = True
        if not changed:
            break
    for arms, name in re.findall(r'(Self :: \w+(?:\s*\|\s*Self :: \w+)*)\s*=> (?:\w+ :: )?(V\d+_\d+_\d+)', all_text):
        value = tuple(map(int, NAMED.fullmatch(name).groups()))
        for arm in re.findall(r'Self :: (\w+)', arms):
            symbols[arm] = value
    found = []
    for filename, ts in parsed.items():
        statement = []
        brackets = parentheses = 0
        for token, line in ts + [(';', 0)]:
            brackets += (token == '[') - (token == ']')
            parentheses += (token == '(') - (token == ')')
            if token in ('{', '}') or (token in (';', ',') and not brackets and not parentheses):
                text = ' '.join(t for t, _ in statement)
                code = ' '.join(t if not (t.startswith(('"', 'r"', 'r#'))) else '""' for t, _ in statement)
                values = [tuple(map(int, m.groups())) for m in NAMED.finditer(code)]
                # A bare "x.y" string or [x, y] array is a version only in a statement that talks
                # about versions; otherwise it is a numeric default such as a "0.44" k-factor.
                if re.search(r'(?i)version|solidcraft|\btarget\b|\bSelf\b|\bV\d+_', text):
                    values += [tuple(int(n or 0) for n in m.groups()) for m in STRING.finditer(text)]
                    values += [tuple(int(n or 0) for n in m.groups()) for m in ARRAY.finditer(code)]
                values += [symbols[t] for t, _ in statement if t in symbols]
                if values:
                    # One entry per distinct release within the statement.
                    for value in sorted(set(values)):
                        key = f'{filename} | {policy.version(value)} | {text}'
                        found.append((key, value, filename, statement[0][1]))
                statement = []
            else:
                statement.append((token, line))
    return found


def allowlines(text):
    return Counter(line for line in text.splitlines() if line and not line.startswith('#'))


def check(current, base, allowed, old_allowed, oldest, bootstrap=False):
    errors = []
    baseline = Counter(row[0] for row in base)
    additions = allowed - old_allowed
    # First introduction must list only gates demonstrably present in the base tree.
    if bootstrap:
        additions -= baseline
    for key in additions.elements():
        errors.append(f'{ALLOW}: allowlist only shrinks; added {key}')
    budget = baseline.copy()
    for key, value, filename, line in current:
        if budget[key]:
            budget[key] -= 1
        elif value < oldest:
            errors.append(f'{filename}:{line}: gate {key.split(" | ", 2)[2]} targets '
                          f'{policy.version(value)}, older than {policy.version(oldest)}. Rule: "{RULE}"')
    return errors


def git(root, *args):
    return subprocess.check_output(['git', '-C', str(root), *args], text=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=policy.ROOT)
    parser.add_argument('--base', default='HEAD', help='PR/merge-group base SHA; HEAD for unstaged local edits')
    parser.add_argument('--releases', type=Path, help='saved release metadata for offline checks')
    args = parser.parse_args()
    root = args.root.resolve()
    _, oldest, _ = policy.window(policy.releases(args.releases))
    base_names = git(root, 'ls-tree', '-r', '--name-only', args.base, '--', TREE, ALLOW).splitlines()
    base_files = {p: git(root, 'show', f'{args.base}:{p}') for p in base_names if p.endswith('.rs') and '/tests/' not in p}
    current_files = {p.relative_to(root).as_posix(): p.read_text() for p in (root / TREE / 'src').rglob('*.rs')}
    old_allowed = allowlines(git(root, 'show', f'{args.base}:{ALLOW}')) if ALLOW in base_names else Counter()
    errors = check(inventory(current_files), inventory(base_files),
                   allowlines((root / ALLOW).read_text()), old_allowed, oldest, ALLOW not in base_names)
    if errors:
        print('\n'.join(errors), file=sys.stderr)
        return 1
    print(f'SolidCraft gates pass (oldest in window: {policy.version(oldest)}; opens-in floor: 0.18.2).')
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        sys.exit(f'version gates: {error}')
