#!/usr/bin/env python3
"""Validate Gherkin syntax and story links. This does not execute application steps."""
from pathlib import Path
import re
import sys

from ui_catalog import ROOT, load_catalog


def scenarios(children, background=()):
    local = tuple(step for child in children for step in child.get('background', {}).get('steps', []))
    for child in children:
        if 'scenario' in child:
            yield child['scenario'], background + local
        elif 'rule' in child:
            yield from scenarios(child['rule']['children'], background + local)


def validate(directory):
    try:
        from gherkin.parser import Parser
        from gherkin.pickles.compiler import Compiler
    except ImportError as error:
        raise ValueError('Install specs/requirements.txt in a development virtual environment first') from error
    files = sorted(directory.rglob('*.feature'))
    if not files:
        raise ValueError('No Gherkin feature files found')
    identifiers = {}
    errors = []
    examples = 0
    for path in files:
        try:
            document = Parser().parse(path.read_text())
            feature = document.get('feature')
            if not feature or feature['language'] != 'en':
                raise ValueError('Expected one English Feature')
            entries = list(scenarios(feature['children']))
            if not entries:
                raise ValueError('Feature has no scenarios')
            for scenario, background in entries:
                tags = [tag['name'] for tag in scenario['tags']]
                ids = [tag[1:] for tag in tags if re.fullmatch(r'@MRDE-[A-Z]+-\d{3}', tag)]
                if len(ids) != 1:
                    raise ValueError(f'{scenario["name"]}: expected one stable @MRDE-AREA-001 ID')
                identifier = ids[0]
                if identifier in identifiers:
                    raise ValueError(f'Duplicate scenario ID {identifier}')
                identifiers[identifier] = path
                steps = [*background, *scenario['steps']]
                keywords = {step['keyword'].strip() for step in steps}
                if not {'Given', 'When', 'Then'} <= keywords:
                    raise ValueError(f'{identifier}: requires context, action and observable outcome')
                if scenario['keyword'] == 'Scenario Outline' and not scenario['examples']:
                    raise ValueError(f'{identifier}: outline has no examples')
                for group in scenario['examples']:
                    header = group.get('tableHeader', {}).get('cells', [])
                    rows = group.get('tableBody', [])
                    names = [cell['value'] for cell in header]
                    if not rows or len(set(names)) != len(names):
                        raise ValueError(f'{identifier}: empty examples or duplicate columns')
                    if any(len(row['cells']) != len(header) for row in rows):
                        raise ValueError(f'{identifier}: examples row width differs from header')
                    placeholders = set(re.findall(r'<([^<>]+)>', '\n'.join(s['text'] for s in scenario['steps'])))
                    if not placeholders <= set(names):
                        raise ValueError(f'{identifier}: an outline parameter has no example column')
            examples += len(Compiler().compile({**document, 'uri': str(path)}))
        except Exception as error:
            errors.append(f'{path.relative_to(ROOT)}: {error}')
    for story in load_catalog():
        missing = set(story['contracts']) - identifiers.keys()
        if missing:
            errors.append(f'{story["id"]}: unknown scenario IDs {sorted(missing)}')
    if errors:
        raise ValueError('\n'.join(errors))
    return len(files), len(identifiers), examples


def main():
    files, contracts, cases = validate(ROOT / 'specs/features')
    print(f'Validated {files} feature files, {contracts} scenario contracts and {cases} expanded cases.')
    print('Story references resolve. No application steps were executed.')


if __name__ == '__main__':
    try:
        main()
    except ValueError as error:
        print(error, file=sys.stderr)
        raise SystemExit(1) from error
