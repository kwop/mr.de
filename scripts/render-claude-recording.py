#!/usr/bin/env python3
"""Render a recorded Claude --output-format stream-json response for documentation.

This does not run Claude and does not imitate its interactive UI. Review the synthetic
session before publishing; only the final response and radar argv are exported.
"""
import argparse
import html
import json
from pathlib import Path
import textwrap


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('recording', type=Path)
    parser.add_argument('output', type=Path, help='New output directory')
    args = parser.parse_args()
    events = [json.loads(line) for line in args.recording.read_text().splitlines() if line]
    calls = [block['input']['command'] for event in events if event.get('type') == 'assistant'
             for block in event.get('message', {}).get('content', [])
             if block.get('type') == 'tool_use' and block.get('name') == 'Bash'
             and block.get('input', {}).get('command', '').startswith('mrde-radar ')]
    result = next((event for event in reversed(events) if event.get('type') == 'result'), {})
    if not calls or result.get('subtype') != 'success' or not result.get('result'):
        raise SystemExit('A completed recording containing a real radar invocation is required')
    response = result['result']
    args.output.mkdir(parents=True, exist_ok=False)
    (args.output / 'claude-radar-response.md').write_text(response + '\n')
    lines = ['RECORDED CLAUDE CODE RESPONSE · synthetic repository', '', '$ ' + calls[-1], '']
    lines.extend(response.splitlines())
    wrapped = []
    for line in lines:
        wrapped.extend(textwrap.wrap(line, width=132, replace_whitespace=False,
                                     drop_whitespace=False) or [''])
    height = len(wrapped) * 21 + 48
    svg = [f'<svg xmlns="http://www.w3.org/2000/svg" width="1240" height="{height}">',
           '<title>Recorded Claude Code radar response, not an interactive UI screenshot</title>',
           '<rect width="100%" height="100%" rx="12" fill="#141518"/>',
           '<g font-family="Menlo,DejaVu Sans Mono,monospace" font-size="14" xml:space="preserve">']
    for index, line in enumerate(wrapped):
        color = '#8ab4f8' if line.startswith(('#', '$', 'RECORDED')) else '#eef0f4'
        svg.append(f'<text x="24" y="{32 + index * 21}" fill="{color}">{html.escape(line)}</text>')
    svg.append('</g></svg>\n')
    (args.output / 'claude-radar-session.svg').write_text(''.join(svg))
    print(args.output)


if __name__ == '__main__':
    main()
