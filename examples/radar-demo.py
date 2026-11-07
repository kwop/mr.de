#!/usr/bin/env python3
"""Create a generic two-commit radar fixture. Never modifies an existing repository."""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile


def create(directory: Path):
    directory.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1')

    def git(*args):
        subprocess.run(['git', '-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgsign=false',
                        *args], cwd=directory, env=env, check=True, capture_output=True)

    def write(path, contents):
        target = directory / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(contents)

    git('init', '-q', '--template=', '--initial-branch=main')
    git('config', 'user.name', 'Demo Contributor')
    git('config', 'user.email', 'demo@example.invalid')
    write('src/auth/session.ts', 'export function canRead(role: string): boolean {\n  return role === "reader";\n}\n')
    write('src/cart.ts', 'export function total(prices: number[]): number {\n  return prices.reduce((sum, price) => sum + price, 0);\n}\n')
    write('risk.toml', '''autodetect_lint = false
[[sensitive]]
glob = "src/auth/**"
weight = 2.0
''')
    git('add', '.')
    git('commit', '-qm', 'Add session and cart helpers')
    write('src/auth/session.ts', '''export function canRead(role: any, override: boolean): boolean {
  // TODO: replace the temporary override with an audited policy.
  if (override) return true;
  return role === "reader" || role === "admin";
}
''')
    write('src/cart.ts', '''import { canRead } from "./auth/session";
export function total(prices: number[], role: string): number {
  // FIXME: define the rounding rule before production use.
  if (!canRead(role, false)) return 0;
  return prices.reduce((sum, price) => sum + price, 0);
}
''')
    git('add', '.')
    git('commit', '-qm', 'Add an access override and cart policy')
    return directory


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', type=Path, help='New directory; an existing one is refused')
    args = parser.parse_args()
    parent = Path(tempfile.mkdtemp(prefix='mrde-radar-example-')) if args.directory is None else None
    repo = create(args.directory or parent / 'repo')
    print(repo)
    print('Compare the two commits: mrde-radar --repo <directory> --base HEAD~1 --head HEAD')
    print('In Claude Code: /mrde:radar HEAD~1 --head HEAD')
