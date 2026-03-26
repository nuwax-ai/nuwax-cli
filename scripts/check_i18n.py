#!/usr/bin/env python3
import re
import sys
from pathlib import Path
from typing import Dict, Any, List, Set
import yaml

ROOT = Path(__file__).resolve().parent.parent
LOCALES = [ROOT / 'locales' / 'en.yml', ROOT / 'locales' / 'zh-CN.yml', ROOT / 'locales' / 'zh-TW.yml']
SRC_DIRS = [ROOT / 'nuwax-cli' / 'src', ROOT / 'client-core' / 'src']

class DupCheckLoader(yaml.SafeLoader):
    pass

def construct_mapping_no_dups(loader, node, deep=False):
    mapping = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if key in mapping:
            raise yaml.constructor.ConstructorError(
                'while constructing a mapping', node.start_mark,
                f'found duplicate key ({key!r})', key_node.start_mark,
            )
        value = loader.construct_object(value_node, deep=deep)
        mapping[key] = value
    return mapping

DupCheckLoader.add_constructor(
    yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG,
    construct_mapping_no_dups,
)


def load_yaml_strict(path: Path) -> Dict[str, Any]:
    text = path.read_text(encoding='utf-8')
    data = yaml.load(text, Loader=DupCheckLoader)
    if not isinstance(data, dict):
        raise ValueError(f'{path} root must be mapping')
    return data


def flatten(d: Dict[str, Any], prefix: str = '') -> Dict[str, str]:
    out: Dict[str, str] = {}
    for k, v in d.items():
        if k == '_version':
            continue
        key = f'{prefix}.{k}' if prefix else str(k)
        if isinstance(v, dict):
            out.update(flatten(v, key))
        else:
            out[key] = str(v)
    return out


def extract_t_keys() -> Set[str]:
    pat = re.compile(r't!\("([^"]+)"')
    keys: Set[str] = set()
    for src in SRC_DIRS:
        for f in src.rglob('*.rs'):
            text = f.read_text(encoding='utf-8')
            keys.update(pat.findall(text))
    return keys


def placeholders(s: str) -> Set[str]:
    return set(re.findall(r'%\{([^}]+)\}', s))


def main() -> int:
    errors: List[str] = []
    warns: List[str] = []

    locale_flat: Dict[str, Dict[str, str]] = {}
    for p in LOCALES:
        try:
            data = load_yaml_strict(p)
            locale_flat[p.name] = flatten(data)
            print(f'[ok] parse {p.name} ({len(locale_flat[p.name])} keys)')
        except Exception as e:
            errors.append(f'parse failed: {p}: {e}')

    if errors:
        for e in errors:
            print(f'[error] {e}')
        return 1

    raw_t_keys = extract_t_keys()
    key_pat = re.compile(r'^[a-z][a-z0-9_]*(\.[a-z0-9_]+)+$')
    t_keys = {k for k in raw_t_keys if key_pat.match(k)}
    print(f'[ok] extracted t! keys: {len(raw_t_keys)} (i18n-style: {len(t_keys)})')

    zh = locale_flat['zh-CN.yml']
    missing_in_zh = sorted(k for k in t_keys if k not in zh)
    if missing_in_zh:
        errors.append(f'zh-CN missing {len(missing_in_zh)} referenced t! keys (first 40): {missing_in_zh[:40]}')

    # en/zh-TW missing against zh-CN: report only
    for name in ('en.yml', 'zh-TW.yml'):
        miss = sorted(k for k in zh.keys() if k not in locale_flat[name])
        if miss:
            warns.append(f'{name} missing {len(miss)} keys compared to zh-CN (first 40): {miss[:40]}')

    # placeholder consistency for keys common to all locales
    common_keys = set.intersection(*(set(v.keys()) for v in locale_flat.values()))
    mismatch = []
    for k in sorted(common_keys):
        p_en = placeholders(locale_flat['en.yml'][k])
        p_cn = placeholders(locale_flat['zh-CN.yml'][k])
        p_tw = placeholders(locale_flat['zh-TW.yml'][k])
        if p_en != p_cn or p_en != p_tw:
            mismatch.append((k, p_en, p_cn, p_tw))

    if mismatch:
        errors.append(f'placeholder mismatch count: {len(mismatch)} (first 20): {mismatch[:20]}')

    for w in warns:
        print(f'[warn] {w}')
    for e in errors:
        print(f'[error] {e}')

    if errors:
        return 1

    print('[ok] i18n checks passed')
    return 0


if __name__ == '__main__':
    sys.exit(main())
