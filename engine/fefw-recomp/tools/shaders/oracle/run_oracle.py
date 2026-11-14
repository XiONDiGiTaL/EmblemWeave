#!/usr/bin/env python3
"""Offline shader-translation oracle.

Runs both Maxwell->SPIR-V translators over one shader corpus:
  (a) Ryujinx harness (spirv_harness.dll --batch), output in <out>/ryujinx
  (b) smo translator: corpus_to_mxs.py per manifest folder -> <out>/mxs, then mx_batch.exe -> <out>/smo
Then re-validates every .spv either translator wrote with spirv-val (vulkan1.3), independently of their own
validation columns, joins the results by code-block hash and reports per-translator pass counts, per-stage breakdown,
agreement, failure reasons and translator messages. Writes <out>/oracle_report.json and <out>/oracle_report.md.

usage: run_oracle.py --corpus <corpus root> --out <work dir> [--jobs 8] [--expect <expected.json>]
                     [--update-expect] [--spirv-val PATH]

Expect file keys: total, ryujinx_valid, smo_valid. Verdict PASS iff total blocks == total, both independent pass
counts >= the expected values, and for both translators the independent spirv-val count == the translator-claimed
pass count (Ryujinx: status ok/ok-retry; smo: status ok).

Exit codes: 0 run ok (and ORACLE PASS when --expect is given); 1 ORACLE FAIL; 2 tool or setup error (missing tool or
file, a tool exiting abnormally, bad arguments). Translator exit code 1 from mx_batch is normal (blocks failed) and is
not an error; the Ryujinx harness must exit 0.
"""
import argparse
import collections
import concurrent.futures as cf
import datetime
import json
import os
import re
import shutil
import subprocess
import sys
import time

TOOLS = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))  # ...\tools\shaders
HARNESS_DLL = os.path.join(TOOLS, 'build', 'bin', 'Release', 'net10.0', 'spirv_harness.dll')
MX_BATCH = os.path.join(TOOLS, 'build', 'smo', 'mx_batch.exe')
CORPUS_TO_MXS = os.path.join(TOOLS, 'scripts', 'corpus_to_mxs.py')
SDK_BIN = r'C:\VulkanSDK\1.4.363.0\Bin'
DEFAULT_SPIRV_VAL = os.path.join(SDK_BIN, 'spirv-val.exe')
GLSLANG = os.path.join(SDK_BIN, 'glslang.exe')
TARGET_ENV = 'vulkan1.3'
TRANSLATORS = ('ryujinx', 'smo')
LABEL = {'ryujinx': 'Ryujinx', 'smo': 'smo'}
PASS_STATUS = {'ryujinx': ('ok', 'ok-retry'), 'smo': ('ok',)}
EXPECT_KEY = {'ryujinx': 'ryujinx_valid', 'smo': 'smo_valid'}
STAGE_ORDER = ['vertex', 'fragment', 'geometry', 'tess_control', 'tess_eval', 'compute', 'unknown']
GEOM_VARIANTS = ('points', 'lines', 'triangles')
TAIL_LINES = 40
MD_LIST_LIMIT = 25


class ToolError(Exception):
    pass


def tail(text, n=TAIL_LINES):
    return '\n'.join(text.splitlines()[-n:])


def run(cmd, label, log_path, ok_codes=(0,)):
    """Run a tool, keep its full output in log_path, and fail loudly on an exit code outside ok_codes."""
    t0 = time.time()
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, encoding='utf-8', errors='replace')
    except OSError as e:
        raise ToolError(f'{label}: could not start: {e}')
    combined = p.stdout + ('\n' if p.stdout and p.stderr else '') + p.stderr
    with open(log_path, 'w', encoding='utf-8') as f:
        f.write('$ ' + subprocess.list2cmdline(cmd) + '\n' + combined)
    if p.returncode not in ok_codes:
        raise ToolError(f'{label} exited with code {p.returncode} (expected one of {list(ok_codes)}); '
                        f'full log: {log_path}\n--- tail ---\n{tail(combined)}')
    return p.returncode, time.time() - t0


def find_manifest_dirs(corpus):
    dirs = []
    for dp, dn, fn in os.walk(corpus):
        dn.sort()
        if 'manifest.tsv' in fn:
            dirs.append(dp)
    return sorted(dirs)


def corpus_blocks(corpus):
    """Distinct code-block hashes (code_sha1_16) over every manifest.tsv under the corpus root."""
    hashes = set()
    for d in find_manifest_dirs(corpus):
        path = os.path.join(d, 'manifest.tsv')
        with open(path, encoding='utf-8', errors='replace') as f:
            header = f.readline().rstrip('\r\n').split('\t')
            if 'code_sha1_16' not in header:
                raise ToolError(f'{path}: no code_sha1_16 column')
            col = header.index('code_sha1_16')
            for line in f:
                if line.strip():
                    fields = line.rstrip('\r\n').split('\t')
                    if len(fields) <= col:
                        raise ToolError(f'{path}: short row: {line.rstrip()}')
                    hashes.add(fields[col])
    return hashes


def read_tsv(path):
    try:
        with open(path, encoding='utf-8', errors='replace') as f:
            lines = [ln.rstrip('\r\n') for ln in f if ln.strip()]
    except OSError as e:
        raise ToolError(f'cannot read {path}: {e}')
    if not lines:
        raise ToolError(f'{path} is empty')
    head = lines[0].split('\t')
    rows = []
    for ln in lines[1:]:
        fields = ln.split('\t')
        if len(fields) > len(head):
            fields = fields[:len(head) - 1] + ['\t'.join(fields[len(head) - 1:])]
        elif len(fields) < len(head):
            fields += [''] * (len(head) - len(fields))
        rows.append(dict(zip(head, fields)))
    return rows


def load_ryujinx(out_dir):
    rows = {}
    for r in read_tsv(os.path.join(out_dir, 'results.tsv')):
        h = r['hash']
        if h in rows:
            raise ToolError(f'duplicate hash {h} in ryujinx results.tsv')
        rows[h] = {
            'name': h,
            'stage': r['stage'],
            'status': r['status'],
            'messages': [m for m in r['translator_messages'].split(' | ') if m],
            'error': r['error'].strip(),
        }
    return rows


def load_smo(out_dir):
    rows = {}
    for r in read_tsv(os.path.join(out_dir, 'results.tsv')):
        name = r['name']
        h = name.split('_', 1)[0]
        if h in rows:
            raise ToolError(f'duplicate block {h} in smo results.tsv')
        rows[h] = {
            'name': name,
            'stage': r['stage'],
            'status': r['status'],
            'messages': [],
            'error': r['error'].strip(),
        }
    return rows


def spv_key(fn):
    """'<hash>.spv' (Ryujinx) or '<hash>_<stage>[.<geometry variant>].spv' (smo) -> (hash, base name)."""
    base = fn[:-len('.spv')]
    for v in GEOM_VARIANTS:
        if base.endswith('.' + v):
            base = base[:-(len(v) + 1)]
    return base.split('_', 1)[0], base


def spirv_val(tool, path):
    p = subprocess.run([tool, '--target-env', TARGET_ENV, path], capture_output=True, text=True,
                       encoding='utf-8', errors='replace')
    text = p.stdout + p.stderr
    if p.returncode == 0:
        return True, ''
    if p.returncode == 1:
        first = next((ln.strip() for ln in text.splitlines() if ln.startswith('error')), text.strip()[:200])
        return False, first
    raise ToolError(f'spirv-val exited with code {p.returncode} on {path}\n--- tail ---\n{tail(text, 10)}')


def validate_all(paths, tool, jobs):
    results = {}
    with cf.ThreadPoolExecutor(max_workers=jobs) as ex:
        futures = {ex.submit(spirv_val, tool, p): p for p in paths}
        for fu in cf.as_completed(futures):
            results[futures[fu]] = fu.result()
    return results


def normalize(s):
    s = re.sub(r'0x[0-9A-Fa-f]+', '0x#', s)
    return re.sub(r'\b\d+\b', '#', s)


def group_texts(pairs):
    """pairs of (text, block hash) -> [{text, blocks, example}] most common first; numbers folded together."""
    members = collections.defaultdict(set)
    example = {}
    for text, h in pairs:
        if not text:
            continue
        k = normalize(text)
        members[k].add(h)
        example.setdefault(k, text)
    out = [{'text': k, 'blocks': len(v), 'example': example[k]} for k, v in members.items()]
    out.sort(key=lambda d: (-d['blocks'], d['text']))
    return out


def norm_stage(s):
    s = (s or '').strip().lower()
    s = {'tessellationcontrol': 'tess_control', 'tessellationevaluation': 'tess_eval'}.get(s, s)
    return s if s in STAGE_ORDER else 'unknown'


def analyse(corpus_hashes, rows, files):
    warnings = []
    for t in TRANSLATORS:
        extra_rows = set(rows[t]) - corpus_hashes
        extra_files = set(files[t]) - corpus_hashes
        missing = corpus_hashes - set(rows[t])
        if extra_rows:
            warnings.append(f'{LABEL[t]}: {len(extra_rows)} result row(s) for hashes not in the corpus')
        if extra_files:
            warnings.append(f'{LABEL[t]}: {len(extra_files)} SPIR-V hash(es) not in the corpus (not counted)')
        if missing:
            warnings.append(f'{LABEL[t]}: no result row for {len(missing)} corpus block(s) '
                            f'(counted as not translated)')

    blocks = []
    for h in sorted(corpus_hashes):
        stages = {norm_stage(rows[t][h]['stage']) for t in TRANSLATORS if h in rows[t]}
        if len(stages) > 1:
            warnings.append(f'stage disagrees for {h}: ' + ', '.join(
                f'{LABEL[t]} {norm_stage(rows[t][h]["stage"])}' for t in TRANSLATORS if h in rows[t]))
        b = {'hash': h, 'stage': sorted(stages)[0] if stages else 'unknown'}
        for t in TRANSLATORS:
            r = rows[t].get(h)
            fl = files[t].get(h, [])
            b[t] = {
                'status': r['status'] if r else 'no-row',
                'translated': len(fl) > 0,
                'claimed': bool(r) and r['status'] in PASS_STATUS[t],
                'valid': len(fl) > 0 and all(ok for _, ok, _ in fl),
                'spv_files': [fn for fn, _, _ in fl],
                'spirv_val_errors': [msg for _, ok, msg in fl if not ok],
            }
        blocks.append(b)

    summary = {}
    for t in TRANSLATORS:
        total = len(blocks)
        translated = sum(b[t]['translated'] for b in blocks)
        claimed = sum(b[t]['claimed'] for b in blocks)
        valid = sum(b[t]['valid'] for b in blocks)
        stages = {}
        for st in STAGE_ORDER:
            sub = [b for b in blocks if b['stage'] == st]
            if sub:
                stages[st] = {'blocks': len(sub),
                              'translated': sum(b[t]['translated'] for b in sub),
                              'valid': sum(b[t]['valid'] for b in sub)}
        summary[t] = {
            'total_blocks': total,
            'translated': translated,
            'claimed_pass': claimed,
            'valid': valid,
            'pass_rate': round(valid / total, 4) if total else 0.0,
            'claimed_equals_independent': claimed == valid and all(
                b[t]['claimed'] == b[t]['valid'] for b in blocks),
            'status_counts': dict(collections.Counter(b[t]['status'] for b in blocks)),
            'per_stage': stages,
            'mismatches': [b['hash'] for b in blocks if b[t]['claimed'] != b[t]['valid']],
        }

    agree = collections.Counter()
    for b in blocks:
        a, s = b['ryujinx']['valid'], b['smo']['valid']
        agree['pass_both' if a and s else 'only_ryujinx' if a else 'only_smo' if s else 'neither'] += 1

    # mx_batch groups reasons by the text before " (" (the detail in parentheses stays in results.tsv)
    smo_failures = group_texts(
        ((r['error'] or r['status']).split(' (')[0], h)
        for h, r in rows['smo'].items() if r['status'] not in PASS_STATUS['smo'] and r['status'] != 'skipped')
    ryu_messages = group_texts((m, h) for h, r in rows['ryujinx'].items() for m in r['messages'])
    # ok-retry rows keep the attempt-1 exception text in their error column (harness Translate())
    ryu_crashes = group_texts(
        ((f"{r['status']}: {r['error']}" if r['status'] != 'ok-retry' else f"attempt 1 threw, retry ok: {r['error']}"), h)
        for h, r in rows['ryujinx'].items()
        if r['status'] in ('exception', 'timeout', 'no-output') or (r['status'] == 'ok-retry' and r['error']))
    spirv_fail = {t: group_texts((msg, b['hash']) for b in blocks for msg in b[t]['spirv_val_errors'])
                  for t in TRANSLATORS}
    return {'blocks': blocks, 'summary': summary, 'agreement': dict(agree), 'warnings': warnings,
            'smo_failure_reasons': smo_failures, 'ryujinx_messages': ryu_messages,
            'ryujinx_crashes': ryu_crashes, 'spirv_val_failures': spirv_fail}


def verdict_for(total, summary, expect):
    if expect is None:
        return {'checked': False, 'result': None, 'reasons': [], 'expected': None}
    reasons = []
    if total != expect['total']:
        reasons.append(f"total blocks {total} != expected {expect['total']}")
    for t in TRANSLATORS:
        got = summary[t]['valid']
        exp = expect[EXPECT_KEY[t]]
        if got < exp:
            reasons.append(f"{LABEL[t]} spirv-val pass {got} < expected {exp}")
        if summary[t]['claimed_pass'] != got:
            reasons.append(f"{LABEL[t]}: independent spirv-val pass {got} != translator-claimed pass "
                           f"{summary[t]['claimed_pass']}")
    return {'checked': True, 'result': 'PASS' if not reasons else 'FAIL', 'reasons': reasons, 'expected': expect}


def pct(n, d):
    return f'{100.0 * n / d:.1f}%' if d else 'n/a'


def to_markdown(rep):
    s = rep['summary']
    L = ['# Shader translation oracle', '',
         f"Corpus: `{rep['corpus']}`  ",
         f"Generated: {rep['generated']}, wall time {rep['wall_seconds']:.0f} s, jobs {rep['jobs']}, "
         f"spirv-val `{rep['spirv_val']}` --target-env {TARGET_ENV}",
         '', f"Distinct code blocks in corpus: **{rep['total_blocks']}**", '',
         '## Per translator', '',
         '| translator | blocks | translated (has SPIR-V) | claimed pass | spirv-val pass (independent) | pass rate | claimed == independent |',
         '|---|---:|---:|---:|---:|---:|:---:|']
    for t in TRANSLATORS:
        x = s[t]
        L.append(f"| {LABEL[t]} | {x['total_blocks']} | {x['translated']} | {x['claimed_pass']} | {x['valid']} | "
                 f"{pct(x['valid'], x['total_blocks'])} | {'yes' if x['claimed_equals_independent'] else 'NO'} |")
    L += ['', '## Per stage (independent spirv-val pass / blocks)', '',
          '| stage | blocks | Ryujinx translated | Ryujinx pass | smo translated | smo pass |',
          '|---|---:|---:|---:|---:|---:|']
    for st in STAGE_ORDER:
        if st not in s['ryujinx']['per_stage']:
            continue
        r, m = s['ryujinx']['per_stage'][st], s['smo']['per_stage'][st]
        L.append(f"| {st} | {r['blocks']} | {r['translated']} | {r['valid']} | {m['translated']} | {m['valid']} |")
    a = rep['agreement']
    L += ['', '## Agreement (independent spirv-val pass)', '', '| category | blocks |', '|---|---:|',
          f"| pass both | {a.get('pass_both', 0)} |",
          f"| only Ryujinx | {a.get('only_ryujinx', 0)} |",
          f"| only smo | {a.get('only_smo', 0)} |",
          f"| neither | {a.get('neither', 0)} |"]
    L += ['', '## smo failure reasons (mx_batch status, blocks)', '', '| blocks | reason |', '|---:|---|']
    for g in rep['smo_failure_reasons'][:MD_LIST_LIMIT]:
        L.append(f"| {g['blocks']} | {g['example'][:160]} |")
    if not rep['smo_failure_reasons']:
        L.append('| 0 | none |')
    L += ['', '## Ryujinx translator messages (blocks reporting them)', '', '| blocks | message |', '|---:|---|']
    for g in rep['ryujinx_messages'][:MD_LIST_LIMIT]:
        L.append(f"| {g['blocks']} | {g['example'][:160]} |")
    if not rep['ryujinx_messages']:
        L.append('| 0 | none |')
    L += ['', '## Translator crashes, exceptions and retries', '',
          f"Ryujinx statuses: {', '.join(f'{k} {v}' for k, v in sorted(s['ryujinx']['status_counts'].items()))}",
          f"smo statuses: {', '.join(f'{k} {v}' for k, v in sorted(s['smo']['status_counts'].items()))}",
          f"Process exits: " + ', '.join(f"{k} {v['exit_code']}" for k, v in rep['steps'].items() if 'exit_code' in v),
          '']
    if rep['ryujinx_crashes']:
        L += ['| blocks | Ryujinx exception, timeout or retry (first example) |', '|---:|---|']
        for g in rep['ryujinx_crashes'][:MD_LIST_LIMIT]:
            L.append(f"| {g['blocks']} | {g['example'][:200]} |")
    else:
        L.append('No Ryujinx exceptions, timeouts or retries.')
    L += ['', '## spirv-val failures (independent run, by message)', '']
    for t in TRANSLATORS:
        groups = rep['spirv_val_failures'][t]
        L.append(f"{LABEL[t]}: " + ('none' if not groups else ''))
        for g in groups[:MD_LIST_LIMIT]:
            L.append(f"- {g['blocks']} block(s): {g['example'][:160]}")
    L += ['', '## Warnings', '']
    L += [f'- {w}' for w in rep['warnings']] or ['none']
    v = rep['verdict']
    L += ['', '## Verdict', '']
    if not v['checked']:
        L.append('No expect file given: numbers only.')
    else:
        exp = v['expected']
        L.append(f"Expected: total {exp['total']}, Ryujinx {exp['ryujinx_valid']}, smo {exp['smo_valid']}  ")
        L.append(f"**{v['result']}**" + (': ' + '; '.join(v['reasons']) if v['reasons'] else ''))
    return '\n'.join(L) + '\n'


def load_expect(path):
    try:
        with open(path, encoding='utf-8') as f:
            e = json.load(f)
    except (OSError, ValueError) as err:
        raise ToolError(f'cannot read expect file {path}: {err}')
    for k in ('total', 'ryujinx_valid', 'smo_valid'):
        if not isinstance(e.get(k), int):
            raise ToolError(f'expect file {path} lacks integer key "{k}"')
    return e


def run_oracle(a):
    t_start = time.time()
    out = os.path.abspath(a.out)
    if a.dksh:
        corpus = os.path.join(out, 'corpus')
        os.makedirs(corpus, exist_ok=True)
        r = subprocess.run([sys.executable, '-I', os.path.join(TOOLS, 'scripts', 'dksh_to_corpus.py'), os.path.abspath(a.dksh),
                            '--out', os.path.join(corpus, 'dksh')], capture_output=True, text=True)
        if r.returncode:
            raise ToolError('dksh_to_corpus.py failed: ' + (r.stdout + r.stderr)[-500:])
    elif a.corpus:
        corpus = os.path.abspath(a.corpus)
    else:
        raise ToolError('give --corpus or --dksh')
    spirv_val_path = os.path.abspath(a.spirv_val)
    for path, what in ((HARNESS_DLL, 'Ryujinx harness dll'), (MX_BATCH, 'mx_batch.exe'),
                       (CORPUS_TO_MXS, 'corpus_to_mxs.py'), (spirv_val_path, 'spirv-val'), (GLSLANG, 'glslang')):
        if not os.path.isfile(path):
            raise ToolError(f'missing {what}: {path}')
    if not os.path.isdir(corpus):
        raise ToolError(f'corpus folder not found: {corpus}')
    if a.jobs < 1:
        raise ToolError('--jobs must be at least 1')
    if a.update_expect and not a.expect:
        raise ToolError('--update-expect needs --expect <file>')
    manifests = find_manifest_dirs(corpus)
    if not manifests:
        raise ToolError(f'no manifest.tsv under {corpus}')
    expect_in = None
    if a.expect and not a.update_expect:
        expect_in = load_expect(os.path.abspath(a.expect))

    corpus_hashes = corpus_blocks(corpus)
    os.makedirs(out, exist_ok=True)
    logs = os.path.join(out, 'logs')
    ryu_dir, mxs_dir, smo_dir = (os.path.join(out, d) for d in ('ryujinx', 'mxs', 'smo'))
    for d in (ryu_dir, mxs_dir, smo_dir, logs):
        shutil.rmtree(d, ignore_errors=True)
    os.makedirs(logs)

    steps = {}
    print(f'[1/4] Ryujinx harness on {len(corpus_hashes)} distinct blocks ...', flush=True)
    rc, sec = run(['dotnet', HARNESS_DLL, '--batch', corpus, '--out', ryu_dir, '--jobs', str(a.jobs),
                   '--spirv-val', spirv_val_path, '--target-env', TARGET_ENV],
                  'Ryujinx harness', os.path.join(logs, 'ryujinx.log'))
    steps['ryujinx_harness'] = {'exit_code': rc, 'seconds': round(sec, 1)}

    print(f'[2/4] smo: corpus_to_mxs on {len(manifests)} manifest folder(s) ...', flush=True)
    skipped = []
    t0 = time.time()
    codes = []
    for i, d in enumerate(manifests):
        log = os.path.join(logs, f'corpus_to_mxs_{i}.log')
        rc, _ = run([sys.executable, CORPUS_TO_MXS, d, '--out', mxs_dir], f'corpus_to_mxs {d}', log, ok_codes=(0, 1))
        codes.append(rc)
        with open(log, encoding='utf-8', errors='replace') as f:
            skipped += [f'{os.path.relpath(d, corpus)}: {ln.strip()}' for ln in f if ln.lstrip().startswith('SKIPPED')]
    steps['corpus_to_mxs'] = {'exit_codes': codes, 'seconds': round(time.time() - t0, 1)}

    print('[3/4] smo: mx_batch.exe ...', flush=True)
    rc, sec = run([MX_BATCH, mxs_dir, '--out', smo_dir, '--jobs', str(a.jobs), '--glslang', GLSLANG,
                   '--spirv-val', spirv_val_path], 'smo mx_batch', os.path.join(logs, 'smo.log'), ok_codes=(0, 1))
    steps['smo_mx_batch'] = {'exit_code': rc, 'seconds': round(sec, 1)}

    print('[4/4] independent spirv-val re-validation and join ...', flush=True)
    rows = {'ryujinx': load_ryujinx(ryu_dir), 'smo': load_smo(smo_dir)}
    spv = []   # (translator, hash, filename, path)
    for t, d in (('ryujinx', ryu_dir), ('smo', smo_dir)):
        for fn in sorted(os.listdir(d)):
            if fn.endswith('.spv'):
                h, _ = spv_key(fn)
                spv.append((t, h, fn, os.path.join(d, fn)))
    t0 = time.time()
    checked = validate_all([p for _, _, _, p in spv], spirv_val_path, a.jobs)
    steps['spirv_val_independent'] = {'files': len(spv), 'seconds': round(time.time() - t0, 1)}
    files = {t: collections.defaultdict(list) for t in TRANSLATORS}
    for t, h, fn, p in spv:
        ok, msg = checked[p]
        files[t][h].append((fn, ok, msg))

    res = analyse(corpus_hashes, rows, files)
    total = len(corpus_hashes)
    warnings = res['warnings'] + ([f'smo input skipped by corpus_to_mxs: {x}' for x in skipped])
    rep = {
        'generated': datetime.datetime.now().isoformat(timespec='seconds'),
        'corpus': corpus, 'out': out, 'jobs': a.jobs, 'spirv_val': spirv_val_path, 'target_env': TARGET_ENV,
        'wall_seconds': round(time.time() - t_start, 1), 'steps': steps,
        'total_blocks': total,
        'summary': res['summary'],
        'agreement': res['agreement'],
        'stages_note': 'stage taken from Ryujinx result, else smo',
        'smo_failure_reasons': res['smo_failure_reasons'],
        'ryujinx_messages': res['ryujinx_messages'],
        'ryujinx_crashes': res['ryujinx_crashes'],
        'spirv_val_failures': res['spirv_val_failures'],
        'warnings': warnings,
        'blocks': res['blocks'],
    }

    expect = expect_in
    if a.update_expect:
        expect = {'total': total, 'ryujinx_valid': res['summary']['ryujinx']['valid'],
                  'smo_valid': res['summary']['smo']['valid']}
        with open(os.path.abspath(a.expect), 'w', encoding='utf-8', newline='\n') as f:
            json.dump(expect, f, indent=2)
            f.write('\n')
        print(f'expect file written: {os.path.abspath(a.expect)} = {expect}', flush=True)
    rep['verdict'] = verdict_for(total, res['summary'], expect)

    with open(os.path.join(out, 'oracle_report.json'), 'w', encoding='utf-8') as f:
        json.dump(rep, f, indent=2, ensure_ascii=False)
    md = to_markdown(rep)
    with open(os.path.join(out, 'oracle_report.md'), 'w', encoding='utf-8', newline='\n') as f:
        f.write(md)
    print(md)
    print(f"report: {os.path.join(out, 'oracle_report.md')}")
    print(f"wall time {rep['wall_seconds']} s")

    v = rep['verdict']
    if not v['checked']:
        return 0
    if v['result'] == 'PASS':
        print('ORACLE PASS')
        return 0
    print('ORACLE FAIL: ' + '; '.join(v['reasons']))
    return 1


def main(argv=None):
    ap = argparse.ArgumentParser(description='Offline shader-translation oracle (Ryujinx harness vs smo mx_batch).')
    ap.add_argument('--corpus', help='corpus root (folders holding manifest.tsv + code/ctrl blobs)')
    ap.add_argument('--dksh', help='folder of .dksh files (uam output): converted with scripts/dksh_to_corpus.py into <out>/corpus first')
    ap.add_argument('--out', required=True, help='work dir for ryujinx/, mxs/, smo/, logs/ and the reports')
    ap.add_argument('--jobs', type=int, default=8)
    ap.add_argument('--expect', help='expected.json with keys total, ryujinx_valid, smo_valid')
    ap.add_argument('--update-expect', action='store_true', help='write the current numbers into --expect')
    ap.add_argument('--spirv-val', default=DEFAULT_SPIRV_VAL)
    a = ap.parse_args(argv)
    try:
        return run_oracle(a)
    except (ToolError, OSError) as e:
        print(f'ORACLE ERROR: {e}', file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
