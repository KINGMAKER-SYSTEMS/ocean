"""Select build components; uncertain diffs conservatively build both."""
import os
import subprocess


def build_scope(paths):
    ocean = surface = False
    for path in paths:
        if not path:
            continue
        # Only repository documentation is exempt. Embedded Markdown under
        # source/profile directories still counts as a build input.
        if path.endswith('/AGENTS.md') or path in ('AGENTS.md', 'events.md'):
            continue
        if path.endswith('.md') and ('/docs/' in path or path.startswith('docs/') or '/' not in path):
            continue
        if path.startswith('.github/') or ('/' not in path and not path.endswith('.md')) or path.startswith('.cargo/'):
            ocean = surface = True
        elif path.startswith('platform/ocean-os/'):
            ocean = True
        elif path.startswith('apps/ocean-surface/'):
            surface = True
    return ocean, surface


def main():
    if os.environ.get('EVENT_NAME') == 'workflow_dispatch':
        selected = (True, True)
    else:
        base = os.environ.get('BASE_SHA', '')
        if not base or set(base) == {'0'}:
            selected = (True, True)
        else:
            try:
                diff = subprocess.check_output(['git', 'diff', '--name-only', '-z', base, 'HEAD'])
                selected = build_scope(diff.decode().split('\0'))
            except (subprocess.CalledProcessError, UnicodeError):
                selected = (True, True)
    with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
        for component, enabled in zip(('ocean', 'surface'), selected):
            output.write(f'{component}={str(enabled).lower()}\n')


if __name__ == '__main__':
    main()
