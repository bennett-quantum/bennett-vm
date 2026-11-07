#!/usr/bin/env python3
"""Repository-local structural ratchet; definitions are documented in docs/quality.md."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
MARKER = "<!-- quality-baseline -->"
PROCESS = re.compile(r"\b(?:bennett-[a-z-]+|b-isa-spec)-[a-z0-9]{3}(?:-[a-z0-9]+)?\b|\bshift\s+\d+|\bquality-vm\b|\bmilestone[ -]+\d+", re.I)
LITTER = re.compile(r"(^|/)(__pycache__|LAST-RUN[^/]*|PROGRESS[^/]*|LOCK)(/|$)|\.(pyc|log|darc|backup)$")
# Strings and comments are blanked while preserving line positions.
TOKEN = re.compile(r'//[^\n]*|/\*|r(#+)"[\s\S]*?"\1|r"[^"]*"|"(?:\\[\s\S]|[^"\\])*"|\x27(?:\\.|[^\x27\\\n])\x27')


def scrub(text):
    pieces = []
    pos = 0
    while (match := TOKEN.search(text, pos)) is not None:
        end = match.end()
        if match[0] == "/*":
            depth = 1
            while depth and end < len(text):
                if text.startswith("/*", end):
                    depth += 1
                    end += 2
                elif text.startswith("*/", end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
            if depth:
                raise ValueError("unterminated block comment")
        pieces.extend([text[pos:match.start()], re.sub(r"[^\n]", " ", text[match.start():end])])
        pos = end
    return "".join(pieces) + text[pos:]


def functions(text):
    code = scrub(text)
    result = []
    for match in re.finditer(r"\bfn\s+(\w+)\b", code):
        start = code.find("{", match.end())
        semi = code.find(";", match.end())
        if start < 0 or 0 <= semi < start:
            continue
        depth = 1
        end = start + 1
        while end < len(code) and depth:
            depth += (code[end] == "{") - (code[end] == "}")
            end += 1
        if depth:
            raise ValueError(f"unbalanced function {match[1]}")
        body = code[start + 1:end - 1]
        lines = text.count("\n", match.start(), end) + 1
        statements = [line.strip() for line in body.splitlines() if line.strip()]
        wrapper = len(statements) <= 3 and bool(re.search(r"\w+\s*\(", body)) and not re.search(r"\b(if|match|for|while|loop)\b", body)
        result.append((match[1], lines, int(wrapper)))
    return result


def structural(text):
    code = scrub(text)
    depth = 0
    mass = 0
    for token in re.finditer(r"\b(?:if|else|for|while|loop|match)\b|&&|\|\||=>|[{}]", code):
        word = token[0]
        if word == "{":
            depth += 1
        elif word == "}":
            depth -= 1
        else:
            mass += 1 + max(0, depth - 1)
    fns = functions(text)
    return {
        "over_25": sum(n > 25 for _, n, _ in fns),
        "over_100": sum(n > 100 for _, n, _ in fns),
        "over_300": sum(n > 300 for _, n, _ in fns),
        "max_lines": max((n for _, n, _ in fns), default=0),
        "branch_mass": mass,
        "trivial_wrappers": sum(w for _, _, w in fns),
    }


def measure(root):
    tracked = subprocess.check_output(["git", "-C", str(root), "ls-files", "-z"], text=True).split("\0")
    source = sorted((root / "src").rglob("*.rs"))
    return {
        "files": {str(p.relative_to(root)): structural(p.read_text()) for p in source},
        "litter": sorted(p for p in tracked if LITTER.search(p)),
        "process_identifiers": {
            str(p.relative_to(root)): len(PROCESS.findall(p.read_text()))
            for p in source if PROCESS.search(p.read_text())
        },
    }


def regressions(baseline, current):
    problems = []
    for file, metrics in current["files"].items():
        for metric, value in metrics.items():
            old = baseline["files"].get(file, {}).get(metric, 0)
            if value > old:
                problems.append(f"{file} {metric}: {old} -> {value}")
    for key in ["litter", "process_identifiers"]:
        if key == "litter":
            problems.extend(f"new litter: {p}" for p in set(current[key]) - set(baseline[key]))
        else:
            for file, count in current[key].items():
                if count > baseline[key].get(file, 0):
                    problems.append(f"{file}: process identifiers increased to {count}")
    return problems


class MetricTests(unittest.TestCase):
    def test_literals_do_not_close_functions(self):
        text = 'fn first() { let x = "}"; /* } */\n if true { run(); }\n}\nfn second() { run(); }'
        self.assertEqual([(n, l) for n, l, _ in functions(text)], [("first", 3), ("second", 1)])

    def test_raw_strings_and_nested_comments(self):
        text = 'fn f() {\n let x = r###" } "###; /* outer /* } */ } */\n run();\n}'
        self.assertEqual([(n, l) for n, l, _ in functions(text)], [("f", 4)])

    def test_quote_inside_comment_cannot_consume_a_following_literal(self):
        text = 'fn f() { /* " */ let x = "}";\n work();\n}'
        self.assertEqual([(n, l) for n, l, _ in functions(text)], [("f", 3)])

    def test_branch_and_wrapper_regressions_are_independent(self):
        before = {"files": {"src/a.rs": structural("fn f() { work(); }")}, "litter": [], "process_identifiers": {}}
        after = {"files": {"src/a.rs": structural("fn f() { if ready() { work(); } }")}, "litter": [], "process_identifiers": {}}
        self.assertTrue(any("branch_mass" in p for p in regressions(before, after)))
        after["files"]["src/a.rs"] = structural("fn f() { work(); }\nfn g() { work(); }")
        self.assertTrue(any("trivial_wrappers" in p for p in regressions(before, after)))

    def test_hygiene_is_ratchet_input(self):
        before = {"files": {}, "litter": [], "process_identifiers": {}}
        after = {"files": {}, "litter": ["x.pyc"], "process_identifiers": {"src/x.rs": 1}}
        self.assertEqual(len(regressions(before, after)), 2)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        unittest.main(argv=[__file__])
        return
    result = unittest.TextTestRunner().run(unittest.defaultTestLoader.loadTestsFromTestCase(MetricTests))
    if not result.wasSuccessful():
        raise SystemExit(1)
    doc = ROOT / "docs/quality.md"
    current = measure(ROOT)
    fence = chr(96) * 3
    if args.record:
        prefix = doc.read_text().split(MARKER)[0].rstrip()
        doc.write_text(prefix + "\n\n" + MARKER + "\n" + fence + "json\n" + json.dumps(current, indent=2, sort_keys=True) + "\n" + fence + "\n")
        return
    text = doc.read_text().split(MARKER, 1)[1]
    baseline = json.loads(text.split(fence + "json\n", 1)[1].split(fence, 1)[0])
    problems = regressions(baseline, current)
    if problems:
        raise SystemExit("\n".join(problems))
    print("quality baseline: no regression")


if __name__ == "__main__":
    main()
