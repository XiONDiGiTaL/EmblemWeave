# SPDX-License-Identifier: GPL-3.0-or-later
"""Known-answer tests of the lifter, without Unicorn: the A64 cases ported from rEden's tests.

    python run_ka.py [--build DIR] [--cases FILE] [-v]

Reads the snx_a64p_script array of ScarletNX's tests/a64_ported_cases.h (GPL-3.0-or-later; the cases come from rEden's and
dynarmic's A64 tests). For each case the instructions that RUN(n) executes are lifted by nxrecomp's lifter (one C++
statement per word), compiled with clang-cl (oracle/ka: ka_host.cpp plus the generated ka_cases.cpp), run on the host,
and every expectation is checked. A case that branches among the instructions it executes, or whose memory window cannot
be mapped at its address, is skipped and counted under its reason.

Prints one line per FAIL (and every line with -v), then the summary; exit code 0 iff no case fails.
"""
import argparse
import collections
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))              # recomp/
sys.path.insert(0, ROOT)
from nxrecomp.lift import Lifter, Unimpl   # noqa: E402

DEFAULT_CASES = os.path.normpath(os.path.join(ROOT, '..', 'refs', 'ScarletNX', 'tests', 'a64_ported_cases.h'))
DEFAULT_BUILD = os.path.join(ROOT, 'build', 'ka')
NX_ENV = os.path.join(ROOT, 'scripts', 'nx_env.cmd')
TOOLCHAIN = os.path.join(ROOT, 'cmake', 'clang-cl.cmake')
WINDOW = 0x4000                  # the ported runner's guest memory window
LOW = 0x10000                    # windows below this address cannot be mapped (the ported runner maps them at 0)
M64 = (1 << 64) - 1

# skip categories (the summary counts them)
CAT_BRANCH = 'branch or control transfer among the executed instructions'
CAT_WIN = 'memory window below 0x10000, or none set'
CAT_MAP = 'window cannot be mapped at its address on this host'
CAT_LIT = 'pc-relative literal load reads the code, not the window'
CAT_RELOC = 'PRESET_NCE relocation does not cover this op'
PRESET_BASE = 0x7300000000     # where a PRESET_NCE case's window goes (0 cannot be mapped on Windows)
RELOC_UNCOVERED = ('SETX', 'SETV', 'MEMORY', 'SMEM', 'CX', 'CV', 'CMEM64', 'CMEMSAME')
SELF_BRANCH = 0x14000000       # b .
LITERAL_LOAD_MASK, LITERAL_LOAD = 0x3B000000, 0x18000000   # LDR / LDRSW / PRFM (literal)
MRS_FPSR_MASK, MRS_FPSR = 0xFFFFFFE0, 0xD53B4420          # mrs xt, FPSR
SHORT = ((CAT_BRANCH, 'branch among the executed instructions'), (CAT_WIN, 'window below 0x10000 or none'),
         (CAT_MAP, 'window not mappable'), (CAT_LIT, 'literal load of the code'),
         (CAT_RELOC, 'relocation not covered'))

CONTROL = re.compile(r'\bgoto\b|\breturn\b|\bcall_indirect\b|\bhost_import\b|\bf_[0-9a-fA-F]+\s*\('
                     r'|\bnx_(catch_call|site_\w+|hook_entry)\b')
MEMTOUCH = re.compile(r'\b(rd|wr)(8|16|32|64)\s*\(|\brdv\s*\(|\bwrv\s*\(|__atomic_(load|store|compare|exchange|fetch)'
                      r'|\bstx\w*\s*\(|\bldxp128\s*\(')
MACRO = re.compile(r'\bSNX_A64P_([A-Z0-9_]+)\s*\(')
LIT = re.compile(r'(0[xX][0-9a-fA-F]+|[0-9]+)([uUlL]*)')
STATUSES = ('PASS', 'FAIL', 'SKIP', 'FPSR-NOT-MODELLED')


# ---------------------------------------------------------------- the header
def script_body(text):
    """The text between `snx_a64p_script[] = {` and its END entry, comments removed."""
    text = re.sub(r'/\*.*?\*/', '', text, flags=re.S)
    text = re.sub(r'//[^\n]*', '', text)
    m = re.search(r'snx_a64p_script\[\]\s*=\s*\{', text)
    if not m:
        raise SystemExit('run_ka: snx_a64p_script not found')
    end = text.index('{SNX_A64P_OP_END', m.end())
    return text[m.end():end]


def macro_calls(body):
    """(name, [argument source text]) for every SNX_A64P_<name>( ... ) call, in order. Strings and nested parens respected."""
    pos = 0
    while True:
        m = MACRO.search(body, pos)
        if not m:
            return
        i = m.end()
        depth, args, cur, in_str = 1, [], [], False
        while depth:
            ch = body[i]
            if in_str:
                cur.append(ch)
                if ch == '\\':
                    i += 1
                    cur.append(body[i])
                elif ch == '"':
                    in_str = False
            elif ch == '"':
                in_str = True
                cur.append(ch)
            elif ch == '(':
                depth += 1
                cur.append(ch)
            elif ch == ')':
                depth -= 1
                if depth:
                    cur.append(ch)
            elif ch == ',' and depth == 1:
                args.append(''.join(cur).strip())
                cur = []
            else:
                cur.append(ch)
            i += 1
        last = ''.join(cur).strip()
        if last:
            args.append(last)
        yield m.group(1), args
        pos = i + 1


def ev(s):
    """An integer C literal (hex or decimal, u/l suffixes), or ~ of one: ~0u is 32 bits wide, ~0ull 64."""
    s = s.strip()
    neg = s.startswith('~')
    if neg:
        s = s[1:].strip()
    m = LIT.fullmatch(s)
    if not m:
        raise ValueError('not an integer literal: %r' % s)
    v = int(m.group(1), 16) if m.group(1)[:2].lower() == '0x' else int(m.group(1), 10)
    if neg:
        width = 64 if 'l' in m.group(2).lower() else 32
        v = (~v) & ((1 << width) - 1)
    return v


def build_ops(calls):
    """The macro calls as ops: ('CASE', literal) / ('CODE', word, literal) / (kind, args...)."""
    ops = []
    for name, a in calls:
        if name == 'CASE':
            ops.append(('CASE', a[0]))
        elif name == 'W':
            ops.append(('CODE', ev(a[0]), a[1]))
        elif name == 'SX':
            ops.append(('SETX', ev(a[0]), ev(a[1])))
        elif name == 'SV':
            ops.append(('SETV', ev(a[0]), ev(a[1]), ev(a[2])))
        elif name == 'SFPCR':
            ops.append(('SETFPCR', ev(a[0])))
        elif name == 'SFPSR':
            ops.append(('SETFPSR', ev(a[0])))
        elif name == 'SNZCV':
            ops.append(('SETNZCV', ev(a[0])))
        elif name == 'CLEARV':
            ops.append(('CLEARV',))
        elif name == 'MEMORY':
            ops.append(('MEMORY', ev(a[0]), ev(a[1])))
        elif name == 'SMEM':
            ops.append(('SMEM', ev(a[0]), a[1]))
        elif name == 'RUN':
            ops.append(('RUN', ev(a[0])))
        elif name == 'CX':
            ops.append(('CX', ev(a[0]), ev(a[1]), M64))
        elif name == 'CXM':
            ops.append(('CX', ev(a[0]), ev(a[1]), ev(a[2])))
        elif name == 'CV':
            ops.append(('CV', ev(a[0]), ev(a[1]), ev(a[2])))
        elif name == 'CPC':
            ops.append(('CPC', ev(a[0])))
        elif name == 'CFPSR':
            ops.append(('CFPSR', ev(a[0]), ev(a[1])))
        elif name == 'CQC':                              # SNX_A64P_CFPSR((set) ? 0x08000000u : 0, 0x08000000u)
            ops.append(('CFPSR', 0x08000000 if ev(a[0]) else 0, 0x08000000))
        elif name == 'CNZCV':
            ops.append(('CNZCV', ev(a[0])))
        elif name == 'CMEM64':
            ops.append(('CMEM64', ev(a[0]), ev(a[1])))
        elif name == 'CMEMSAME':
            ops.append(('CMEMSAME', ev(a[0]), ev(a[1]), ev(a[2])))
        elif name == 'PRESET_NCE':
            ops.append(('PRESET',))
        elif name == 'MARK':
            ops.append(('MARK',))
        elif name == 'WX':
            ops.append(('WX', ev(a[0]), ev(a[1])))
        elif name == 'WV':
            ops.append(('WV', ev(a[0]), ev(a[1]), ev(a[2])))
        elif name == 'WMEM':
            ops.append(('WMEM', ev(a[0]), ev(a[1]), ev(a[2])))
        elif name == 'SAME':
            ops.append(('SAME',))
        else:
            raise SystemExit('run_ka: unknown macro SNX_A64P_%s' % name)
    return ops


def load_cases(path):
    with open(path, encoding='utf-8') as f:
        text = f.read()
    cases = []
    for op in build_ops(macro_calls(script_body(text))):
        if op[0] == 'CASE':
            cases.append({'name': op[1], 'ops': []})
        elif cases:
            cases[-1]['ops'].append(op)
        else:
            raise SystemExit('run_ka: an op before the first CASE')
    return cases


# ---------------------------------------------------------------- C text helpers
def hx(v):
    return '0x%xULL' % (v & M64)


def cstr(s):
    out = ['"']
    for ch in s:
        if ch in '"\\':
            out.append('\\' + ch)
        elif ch == '\n':
            out.append('\\n')
        elif ord(ch) < 0x20 or ord(ch) > 0x7e:
            out.append('\\x%02x' % (ord(ch) & 0xff))
        else:
            out.append(ch)
    out.append('"')
    return ''.join(out)


def comment(s):
    return s.replace('*/', '* /').replace('\n', ' ').replace('\r', ' ')


# ---------------------------------------------------------------- the lifter
class Env:
    """The environment the lifter needs (difftest.py's dt_env): no jump tables, a call is a call to f_<target>."""
    wanted = {}
    jt = {}

    def call(self, target):
        return 'f_%x(c);' % target


_lifter = None
_lift_cache = {}


def lift(word, pc):
    """-> (statement, None) or (None, error text). The lifter's own failures are reported, not hidden."""
    global _lifter
    key = (word, pc)
    if key not in _lift_cache:
        if _lifter is None:
            _lifter = Lifter(0, 0x10000, Env())
        try:
            stmt = _lifter.lift(word, pc)
            _lift_cache[key] = (stmt.replace('\n', ' '), None)
        except Unimpl as e:
            _lift_cache[key] = (None, 'Unimpl: %s' % e)
        except Exception as e:                       # any other lifter failure is a lifter failure too
            _lift_cache[key] = (None, '%s: %s' % (type(e).__name__, e))
    return _lift_cache[key]


# ---------------------------------------------------------------- one case
def window_ok(win):
    return win is not None and win >= LOW


def translate(case):
    """-> ('skip', category + detail) | ('error', text) | ('run', body lines, mapped window bases).

    body lines are (C text, info) where info is (word, instruction text) for a lifted statement, else None."""
    ops = case['ops']
    codes = [op for op in ops if op[0] == 'CODE']
    first_state = next((i for i, op in enumerate(ops) if op[0] != 'CODE'), len(ops))
    if any(op[0] == 'CODE' for op in ops[first_state:]):
        return ('error', 'code after the state: the code must come first, from address 0')
    # a PRESET_NCE case: its window at address 0 moves to PRESET_BASE, and the addresses the case holds move with it
    reloc = any(op[0] == 'PRESET' for op in ops)
    off = PRESET_BASE if reloc else 0

    def rb(v):                      # a value the case holds as an address (a window offset 0 < v < WINDOW) in a relocated case
        return off + v if reloc and 0 < v < WINDOW else v

    body, bases = [], []
    if any(op[0] == 'SETFPCR' and op[1] & 0xFFFFFFFF for op in ops):
        body.append(('ka_case_fpcr();', None))     # a non-zero FPCR: a failure here is FPCR-not-honoured
    win = None                      # base of the window the ops use, None when none is set
    fpsr_read = False               # an executed mrs of FPSR came before this point
    for op in ops:
        kind = op[0]
        if kind == 'CODE':
            continue
        if reloc and kind in RELOC_UNCOVERED:
            return ('skip', CAT_RELOC + ' (%s)' % kind)
        if kind == 'PRESET':
            win = PRESET_BASE
            bases.append(PRESET_BASE)
            body.append(('ka_preset(c, %s);' % hx(PRESET_BASE), None))
        elif kind == 'SETX':
            body.append(('c->x[%d] = %s;' % (op[1], hx(op[2])), None))
        elif kind == 'SETV':
            body.append(('c->v[%d].d[0] = %s; c->v[%d].d[1] = %s;' % (op[1], hx(op[2]), op[1], hx(op[3])), None))
        elif kind == 'SETFPCR':
            body.append(('c->fpcr = %s;' % hx(op[1] & 0xFFFFFFFF), None))
        elif kind == 'SETFPSR':
            body.append(('c->fpsr = %s;' % hx(op[1] & 0xFFFFFFFF), None))
        elif kind == 'SETNZCV':
            body.append(('set_nzcv(c, %d);' % ((op[1] >> 28) & 0xFFFFFFFF), None))
        elif kind == 'CLEARV':
            body.append(('memset(c->v, 0, sizeof c->v);', None))
        elif kind == 'MEMORY':
            win = op[1]
            if op[1] < LOW:
                return ('skip', CAT_WIN + ' (MEMORY at %s)' % hx(op[1]))
            if op[1] not in bases:
                bases.append(op[1])
            body.append(('ka_memory(%s, %d);' % (hx(op[1]), op[2]), None))
        elif kind in ('SMEM', 'CMEM64', 'CMEMSAME', 'WMEM', 'SAME'):
            if not window_ok(win):
                return ('skip', CAT_WIN + ' (%s with no MEMORY window before it: implicit window at address 0)' % kind)
            if kind == 'SMEM':
                body.append(('ka_smem(%s, %s);' % (hx(op[1]), op[2]), None))
            elif kind == 'CMEM64':
                body.append(('ka_check_mem64(%s, %s);' % (hx(op[1]), hx(op[2])), None))
            elif kind == 'CMEMSAME':
                body.append(('ka_check_mem_same(%s, %s, %d);' % (hx(op[1]), hx(op[2]), op[3]), None))
            elif kind == 'WMEM':
                if reloc and op[1] >= WINDOW:
                    return ('skip', CAT_RELOC + ' (WMEM outside the window)')
                body.append(('ka_want_mem(%s, %d, %s);' % (hx(op[1] + off), op[2], hx(rb(op[3]))), None))
            else:
                body.append(('ka_check_same(c);', None))
        elif kind == 'RUN':
            n = op[1]
            k = min(n, len(codes))
            # b . (the ported runner's end of execution): the code stops before it, pc stays at its address
            stop = next((i for i in range(k) if codes[i][1] == SELF_BRANCH), k)
            body.append(('ka_run_begin();', None))
            for i in range(stop):
                word, text = codes[i][1], codes[i][2]
                stmt, err = lift(word, 4 * i)
                label = '%d: %08x %s' % (i, word, comment(text))
                if err is not None:
                    body.append(('ka_lifter_gap(%s, %s);' % (cstr('%08x %s' % (word, text)), cstr(err)), None))
                    continue
                if CONTROL.search(stmt):
                    return ('skip', CAT_BRANCH + ' (step %d: %08x %s)' % (i + 1, word, comment(text)))
                if (word & LITERAL_LOAD_MASK) == LITERAL_LOAD:
                    return ('skip', CAT_LIT + ' (step %d: %08x %s)' % (i + 1, word, comment(text)))
                if MEMTOUCH.search(stmt) and not window_ok(win):
                    return ('skip', CAT_WIN + ' (step %d touches memory: %08x %s)' % (i + 1, word, comment(text)))
                if (word & MRS_FPSR_MASK) == MRS_FPSR:
                    fpsr_read = True
                body.append(('{ /* %s */ %s }' % (label, stmt), (word, text)))
            body.append(('ka_run_end(%s);' % hx(4 * stop), None))
        elif kind == 'CX':
            if op[3] == 0x08000000 and fpsr_read:      # reads the QC bit after an mrs of FPSR: an FPSR expectation
                body.append(('ka_check_x_qc(c, %d, %s, %s);' % (op[1], hx(op[2]), hx(op[3])), None))
            else:
                body.append(('ka_check_x(c, %d, %s, %s);' % (op[1], hx(op[2]), hx(op[3])), None))
        elif kind == 'CV':
            body.append(('ka_check_v(c, %d, %s, %s);' % (op[1], hx(op[2]), hx(op[3])), None))
        elif kind == 'CPC':
            body.append(('ka_check_pc(%s);' % hx(op[1]), None))
        elif kind == 'CFPSR':
            body.append(('ka_check_fpsr(c, %s, %s);' % (hx(op[1]), hx(op[2])), None))
        elif kind == 'CNZCV':
            body.append(('ka_check_nzcv(c, %s);' % hx(op[1]), None))
        elif kind == 'MARK':
            body.append(('ka_mark(c);', None))
        elif kind == 'WX':
            body.append(('ka_want_x(%d, %s);' % (op[1], hx(rb(op[2]))), None))
        elif kind == 'WV':
            body.append(('ka_want_v(%d, %s, %s);' % (op[1], hx(op[2]), hx(op[3])), None))
        else:
            raise SystemExit('run_ka: unhandled op %r' % (op,))
    return ('run', body, bases)


def case_text(idx, body, bases):
    lines = ['static bool case_%d(Ctx *c) {' % idx]
    for b in bases:
        lines.append('    if (!ka_map(%s, %d)) return false;' % (hx(b), WINDOW))
    for text, _ in body:
        lines.append('    ' + text)
    lines.append('    return true;')
    lines.append('}')
    return lines


def gen_source(cases):
    """-> (source text, {1-based line: text} of the lifted statements, [skipped case lines])."""
    runs, stmt_lines = [], {}
    out_fns = []
    for idx, case in enumerate(cases):
        t = translate(case)
        name = case['name']                  # the header's own C string literal
        if t[0] == 'skip':
            runs.append('    ka_skip(%s, %s);' % (name, cstr(t[1])))
            continue
        if t[0] == 'error':
            out_fns.append(['static bool case_%d(Ctx *c) {' % idx, '    ka_script_error(%s);' % cstr(t[1]), '    return false;', '}'])
            runs.append('    ka_run(%s, case_%d);' % (name, idx))
            continue
        out_fns.append(case_text(idx, t[1], t[2]))
        runs.append('    ka_run(%s, case_%d);' % (name, idx))
    head = ['/* generated by run_ka.py from the ported A64 cases -- do not edit */',
            '#include <nxrecomp/cxx.h>',
            'typedef bool (*KaFn)(Ctx *);',
            'extern "C" {',
            'void ka_run(const char *name, KaFn fn);',
            'void ka_skip(const char *name, const char *reason);',
            'void ka_script_error(const char *what);',
            'void ka_lifter_gap(const char *insn, const char *why);',
            'bool ka_map(uint64_t base, uint64_t size);',
            'void ka_memory(uint64_t base, int zeros);',
            'void ka_smem(uint64_t addr, const char *text);',
            'void ka_run_begin(void);',
            'void ka_run_end(uint64_t pc);',
            'void ka_check_x(Ctx *c, unsigned n, uint64_t a, uint64_t mask);',
            'void ka_check_v(Ctx *c, unsigned n, uint64_t lo, uint64_t hi);',
            'void ka_check_pc(uint64_t a);',
            'void ka_check_fpsr(Ctx *c, uint64_t a, uint64_t mask);',
            'void ka_check_nzcv(Ctx *c, uint64_t a);',
            'void ka_check_mem64(uint64_t a, uint64_t v);',
            'void ka_check_mem_same(uint64_t a, uint64_t b, unsigned n);',
            'void ka_mark(Ctx *c);',
            'void ka_want_x(unsigned n, uint64_t v);',
            'void ka_want_v(unsigned n, uint64_t lo, uint64_t hi);',
            'void ka_want_mem(uint64_t a, unsigned n, uint64_t v);',
            'void ka_check_same(Ctx *c);',
            'void ka_preset(Ctx *c, uint64_t base);',
            'void ka_case_fpcr(void);',
            'void ka_check_x_qc(Ctx *c, unsigned n, uint64_t a, uint64_t mask);',
            '}']
    lines = list(head)
    for fn in out_fns:
        lines.append('')
        for ln in fn:
            lines.append(ln)
            if ln.startswith('    { /*'):
                stmt_lines[len(lines)] = ln
    lines.append('')
    lines.append('extern "C" void ka_run_all(void) {')
    lines.extend(runs)
    lines.append('}')
    return '\n'.join(lines) + '\n', stmt_lines


# ---------------------------------------------------------------- build and run
def sh(cmd):
    return subprocess.run(['cmd', '/c', NX_ENV] + cmd, capture_output=True, text=True)


def build(work, gen_path, stmt_lines):
    bdir = os.path.join(work, 'cmake')
    gen_dir = os.path.dirname(gen_path)
    r = sh(['cmake', '-S', HERE, '-B', bdir, '-G', 'Ninja', '-DCMAKE_BUILD_TYPE=Release',
            '-DCMAKE_TOOLCHAIN_FILE=' + TOOLCHAIN.replace('\\', '/'), '-DKA_GEN_DIR=' + gen_dir.replace('\\', '/')])
    if r.returncode:
        print(r.stdout[-3000:], r.stderr[-3000:])
        raise SystemExit(2)
    for _attempt in range(6):
        r = sh(['cmake', '--build', bdir, '-j', str(os.cpu_count() or 8), '--', '-k', '0'])
        if r.returncode == 0:
            return os.path.join(bdir, 'ka.exe')
        out = r.stdout + r.stderr
        errs = re.findall(r'ka_cases\.cpp\((\d+),\d+\): error: (.*)', out)
        if not errs:
            print(out[-5000:])
            raise SystemExit(2)
        # a lifted statement that does not compile becomes a reported lifter failure of its own case, not a lost run
        text = open(gen_path, encoding='utf-8').read().split('\n')
        for ln, msg in errs:
            n = int(ln)
            if n in stmt_lines:
                word_text = stmt_lines[n]
                m = re.search(r'/\* (\d+): ([0-9a-f]{8}) (.*?) \*/', word_text)
                insn = '%s %s' % (m.group(2), m.group(3)) if m else '?'
                text[n - 1] = '    ka_lifter_gap(%s, %s);' % (cstr(insn), cstr('the lifted statement does not compile: ' + msg.strip()[:300]))
                stmt_lines.pop(n)
            else:
                print('error outside a lifted statement: ka_cases.cpp:%d: %s' % (n, msg))
                print(out[-3000:])
                raise SystemExit(2)
        with open(gen_path, 'w', encoding='utf-8', newline='\n') as f:
            f.write('\n'.join(text))
    print('build still failing after the repair rounds')
    raise SystemExit(2)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--build', default=DEFAULT_BUILD, help='work directory: gen/ (cases) and cmake/ (build) go here')
    ap.add_argument('--cases', default=DEFAULT_CASES, help='a64_ported_cases.h')
    ap.add_argument('-v', '--verbose', action='store_true', help='print every result line, not only FAIL')
    a = ap.parse_args()
    work = os.path.abspath(a.build)
    gen_dir = os.path.join(work, 'gen')
    os.makedirs(gen_dir, exist_ok=True)
    cases = load_cases(a.cases)
    src, stmt_lines = gen_source(cases)
    gen_path = os.path.join(gen_dir, 'ka_cases.cpp')
    with open(gen_path, 'w', encoding='utf-8', newline='\n') as f:
        f.write(src)
    exe = build(work, gen_path, stmt_lines)
    r = subprocess.run([exe], capture_output=True, text=True)
    results = collections.Counter()
    reasons = collections.Counter()
    lines = []
    fpcr_names = []
    names = sorted((c['name'][1:-1] for c in cases), key=len, reverse=True)   # the header's names, longest first
    for line in r.stdout.splitlines():
        m = re.match(r'(PASS|FAIL|SKIP|FPSR-NOT-MODELLED|FPCR-NOT-HONOURED) (.*)$', line)
        if not m:
            if line.strip():
                lines.append('?? ' + line)
            continue
        status, rest = m.group(1), m.group(2)
        results[status] += 1
        if status == 'SKIP':
            reasons[next((short for cat, short in SHORT if cat in rest), 'other')] += 1
        if status == 'FPCR-NOT-HONOURED':
            fpcr_names.append(next((n for n in names if rest.startswith(n + ': ')), rest.split(': ', 1)[0]))
        if status == 'FAIL' or a.verbose:
            lines.append(line)
    n_cases = len(cases)
    seen = sum(results.values())
    if seen != n_cases:
        results['FAIL'] += n_cases - seen
        lines.append('FAIL <runner>: %d of %d cases have no result line (exit code %d)' % (n_cases - seen, n_cases, r.returncode))
    for line in lines:
        print(line)
    if r.returncode and seen == n_cases:
        print('the runner exited %d' % r.returncode)
    reason_text = ', '.join('%s: %d' % kv for kv in sorted(reasons.items())) or 'none'
    print('ka: %d cases pass, %d fail, %d skipped (%s), %d FPSR-not-modelled, %d FPCR-not-honoured' % (
        results['PASS'], results['FAIL'], results['SKIP'], reason_text, results['FPSR-NOT-MODELLED'],
        results['FPCR-NOT-HONOURED']))
    if fpcr_names:
        print('FPCR-not-honoured (a documented gap: the lifted FP code ignores FPCR): ' + '; '.join(fpcr_names))
    sys.exit(0 if results['FAIL'] == 0 and r.returncode == 0 else 1)


if __name__ == '__main__':
    main()
