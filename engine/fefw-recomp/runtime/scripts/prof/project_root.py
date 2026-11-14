"""The project folder the profiling scripts work in (game/, game-build/, work/ live there).

NXRT_ROOT names it; otherwise it is the nearest parent of this checkout that holds both a runtime/ and a work/ folder.
"""
import os


def project_root():
    e = os.environ.get('NXRT_ROOT')
    if e:
        return e
    d = os.path.dirname(os.path.abspath(__file__))
    while True:
        if os.path.isdir(os.path.join(d, 'runtime')) and os.path.isdir(os.path.join(d, 'work')):
            return d
        p = os.path.dirname(d)
        if p == d:
            return os.getcwd()
        d = p
