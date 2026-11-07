#!/usr/bin/env python3
"""Explore and capture MR.DE's real GPUI views with synthetic preview data."""
import argparse
import html
import json
import os
from pathlib import Path
import re
import signal
import struct
import subprocess
import sys
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parent.parent
CATALOG = ROOT / 'specs/ui/catalog.toml'
ENV_KEYS = {
    'MRDE_SCENARIO_MR', 'MRDE_SCENARIO_FRESH', 'MRDE_SCENARIO_RISK',
    'MRDE_SCENARIO_SCROLL', 'MRDE_SCENARIO_CONTEXT', 'MRDE_SCENARIO_SEARCH',
    'MRDE_LANGUAGE', 'MRDE_SCENARIO_LABEL', 'MRDE_SCENARIO_AXIS', 'MRDE_SCENARIO_FILE', 'MRDE_WINDOW',
}
TABS = {'description', 'changes', 'files', 'review', 'statistics', 'checks',
        'structure', 'history', 'risks', 'visual'}
SCENARIOS = {'queue', 'projects', 'settings', 'settings:github', 'worktrees',
             'settings:languages', 'logs', 'open', 'confirm', 'image', 'image:zoom', 'image:max', 'threads', 'threads:closed'}
SCENARIOS |= {f'{mode}:{tab}' for mode in ('focus', 'detail') for tab in TABS}


def load_catalog():
    data = tomllib.loads(CATALOG.read_text())
    if data.get('version') != 1:
        raise ValueError('Unsupported UI catalog version')
    stories = data.get('story', [])
    seen = set()
    for story in stories:
        key = story['id']
        if not re.fullmatch(r'[a-z][a-z0-9]*(?:[.-][a-z0-9]+)*', key) or key in seen:
            raise ValueError(f'Invalid or duplicate story ID: {key}')
        seen.add(key)
        if story['scenario'] not in SCENARIOS:
            raise ValueError(f'Unsupported preview scenario: {story["scenario"]}')
        if not story.get('title') or not story.get('contracts') or not story.get('inspect'):
            raise ValueError(f'{key}: title, contract IDs and inspection notes are required')
        for name, value in story.get('env', {}).items():
            if name not in ENV_KEYS or not isinstance(value, str):
                raise ValueError(f'{key}: unsupported preview variable {name}')
    if not stories:
        raise ValueError('The UI catalog is empty')
    return stories


def preview_environment(story, snapshot=None):
    # Discard all inherited automation/real-data overrides, including local risk reports.
    env = {k: v for k, v in os.environ.items() if not k.startswith('MRDE_')}
    env.update(MRDE_SCENARIO=story['scenario'], MRDE_WINDOW='1100x900')
    env.update(story.get('env', {}))
    if snapshot is not None:
        env.update(MRDE_SNAPSHOT=str(snapshot), MRDE_SNAPSHOT_DELAY_MS='3000')
    return env


def build_binary(no_build):
    target = ROOT / 'target'
    if not no_build:
        subprocess.run(['cargo', 'build', '--locked', '-p', 'mrde',
                        '--target-dir', str(target)], cwd=ROOT, check=True)
    binary = target / 'debug/mrde'
    if not binary.is_file():
        raise ValueError('No debug executable; run again without --no-build')
    return binary


def new_capture_directory(requested):
    if requested is None:
        return Path(tempfile.mkdtemp(prefix='mrde-ui-catalog-'))
    directory = Path(requested).expanduser().resolve()
    # Never replace an earlier candidate or an approved reference accidentally.
    directory.mkdir(parents=True, exist_ok=False)
    return directory


def run_capture(binary, story, directory, timeout):
    png = directory / f'{story["id"]}.png'
    log = directory / f'{story["id"]}.log'
    with log.open('w') as output:
        process = subprocess.Popen([str(binary), '--ui-preview'], cwd=ROOT,
                                   env=preview_environment(story, png), stdout=output,
                                   stderr=output, start_new_session=True)
        try:
            code = process.wait(timeout=timeout)
        except (subprocess.TimeoutExpired, KeyboardInterrupt):
            # Stop only the process group this capture started; never the user's app.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
            raise
    if code:
        raise ValueError(f'Preview exited with status {code}; see {log.name}')
    if not png.is_file():
        raise ValueError(f'Preview produced no capture; see {log.name}')
    with png.open('rb') as stream:
        header = stream.read(24)
    if len(header) != 24 or header[:8] != b'\x89PNG\r\n\x1a\n':
        raise ValueError(f'Invalid PNG: {png.name}')
    width, height = struct.unpack('>II', header[16:24])
    return {'image': png.name, 'pixels': [width, height]}


def write_gallery(directory, results):
    panels = []
    for entry in results:
        story = entry['story']
        notes = ''.join(f'<li>{html.escape(note)}</li>' for note in story['inspect'])
        if 'image' in entry:
            name = html.escape(entry['image'], quote=True)
            media = f'<a href="{name}"><img src="{name}" alt="{html.escape(story["title"], quote=True)}"></a>'
        else:
            media = f'<p class="failure">Capture failed: {html.escape(entry["error"])}</p>'
        contracts = html.escape(', '.join(story['contracts']))
        panels.append(f'<article><h2>{html.escape(story["id"])}</h2><p>{html.escape(story["title"])}</p>'
                      f'{media}<ul>{notes}</ul><small>{contracts}</small></article>')
    page = '''<!doctype html><html lang="en"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>MR.DE UI catalog</title>
<style>body{font:16px system-ui;margin:32px;background:#141518;color:#eef0f4}
h1{font-size:24px}main{display:grid;grid-template-columns:repeat(auto-fit,minmax(min(440px,100%),1fr));gap:24px}
article{padding:20px;background:#292c32;border:1px solid #535965;border-radius:10px}
img{width:100%;height:auto}small{color:#bbc2ce}li{margin:8px 0}.failure{color:#f08080}</style>
<h1>MR.DE UI catalog</h1><p>Candidate captures, not approved baselines. Click an image to inspect it.
These are screenshots of the native app; the gallery does not render GPUI components.</p><main>'''
    (directory / 'index.html').write_text(page + ''.join(panels) + '</main></html>')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    commands.add_parser('list', help='List stable story IDs')
    commands.add_parser('check', help='Validate catalog structure without launching the app')
    launch = commands.add_parser('open', help='Open an interactive native preview')
    launch.add_argument('story')
    launch.add_argument('--no-build', action='store_true')
    launch.add_argument('--dry-run', action='store_true')
    capture = commands.add_parser('capture', help='Capture selected stories or the entire catalog')
    capture.add_argument('stories', nargs='*')
    capture.add_argument('--output', help='New output directory; defaults to an OS temporary directory')
    capture.add_argument('--no-build', action='store_true')
    capture.add_argument('--timeout', type=int, default=40)
    args = parser.parse_args()
    stories = load_catalog()
    by_id = {story['id']: story for story in stories}
    if args.command == 'list':
        for story in stories:
            print(f'{story["id"]:26} {story["title"]}')
        return 0
    if args.command == 'check':
        print(f'Validated {len(stories)} native story definitions; no app launched.')
        return 0
    selected_ids = [args.story] if args.command == 'open' else args.stories or list(by_id)
    unknown = set(selected_ids) - by_id.keys()
    if unknown:
        raise ValueError(f'Unknown stories: {", ".join(sorted(unknown))}')
    selected = [by_id[key] for key in dict.fromkeys(selected_ids)]
    if args.command == 'open' and args.dry_run:
        env = {k: v for k, v in preview_environment(selected[0]).items() if k.startswith('MRDE_')}
        print(json.dumps({'argv': ['target/debug/mrde', '--ui-preview'], 'env': env}, indent=2))
        return 0
    if sys.platform != 'darwin':
        raise ValueError('The current desktop preview and capture harness require macOS')
    if args.command == 'capture' and not 1 <= args.timeout <= 60:
        raise ValueError('--timeout must be between 1 and 60 seconds per story')
    binary = build_binary(args.no_build)
    if args.command == 'open':
        return subprocess.run([str(binary), '--ui-preview'], cwd=ROOT,
                              env=preview_environment(selected[0])).returncode
    directory = new_capture_directory(args.output)
    results = []
    for story in selected:
        result = {'story': story, 'requested_window': preview_environment(story)['MRDE_WINDOW']}
        try:
            result.update(run_capture(binary, story, directory, args.timeout))
        except (ValueError, subprocess.TimeoutExpired) as error:
            result['error'] = str(error)
        results.append(result)
        print(story['id'], 'FAILED' if 'error' in result else 'captured', flush=True)
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    dirty = bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT))
    (directory / 'manifest.json').write_text(json.dumps({
        'revision': revision, 'dirty': dirty, 'platform': sys.platform,
        'baseline_approved': False, 'captures': results,
    }, indent=2) + '\n')
    write_gallery(directory, results)
    print(f'Gallery: {directory / "index.html"}')
    return int(any('error' in result for result in results))


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        raise SystemExit(str(error)) from error
