"""StringStats against the same computation in plain Python: bit-identical
floats, and int 0 exactly where the Python returns it."""

import math
import random
import string
import operator
from functools import reduce

import pytest

from androguard_rs import StringStats

CHARSETS = ["0123456789abcdef", string.ascii_letters + string.digits + "+/=", string.printable]
ALPHABET = string.digits + string.ascii_lowercase


def distance_histogram(charset):
    if len(charset) <= 1:
        return {0: 1}
    buckets, n = {}, 0
    for a in charset:
        for b in charset:
            d = abs(ord(a) - ord(b))
            buckets[d] = buckets.get(d, 0) + 1
            n += 1
    return {k: v / n for k, v in buckets.items()}


def tables(log_prob):
    charsets = []
    for cs in CHARSETS:
        hist = distance_histogram(cs)
        keys = list(hist)
        values = [hist[k] for k in keys]
        absent = [(0 - v) ** 2 / len(keys) for v in values]
        charsets.append((cs, keys, values, absent))
    normalize = {}
    for cp in range(0x110000):
        low = chr(cp).lower()
        if len(low) == 1 and low in ALPHABET:
            normalize[cp] = ALPHABET.index(low)
    return charsets, normalize


def reference(s, charsets, normalize, log_prob):
    if not s:
        return None
    for cs, keys, values, absent in charsets:
        if set(s) <= set(cs):
            break
    else:
        return None
    n = len(s)
    index = {c: i for i, c in enumerate(cs)}
    counts = {}
    for c in s:
        counts[c] = counts.get(c, 0) + 1
    entropy = 0.0
    for c in sorted(counts, key=index.__getitem__):
        p = float(counts[c]) / n
        entropy = entropy + p * math.log(p, 2)
    entropy = -entropy / n

    if n <= 2:
        seq = 0
    else:
        w = int(math.floor(math.log(n)))
        dist, counter = {}, 0
        for k in range(1, min(w, n - 1) + 1):
            for a, b in zip(s, s[k:]):
                d = abs(ord(b) - ord(a))
                dist[d] = dist.get(d, 0) + 1
            counter += n - k
        pos = {k: i for i, k in enumerate(keys)}
        terms = list(absent)
        for d, c in dist.items():
            if d in pos:
                terms[pos[d]] = (c / counter - values[pos[d]]) ** 2 / len(keys)
        seq = reduce(operator.add, terms, 0)

    kept = [normalize[ord(c)] for c in s if ord(c) in normalize]
    if len(kept) <= 1:
        gib = 0
    else:
        lp = reduce(operator.add, (log_prob[a][b] for a, b in zip(kept, kept[1:])), 0.0)
        gib = math.exp(lp / (len(kept) - 1))
    return (entropy, seq, gib, float(len(cs)))


def same(a, b):
    if a is None or b is None:
        return a is b
    return repr(a) == repr(b) and all(type(x) is type(y) for x, y in zip(a, b))


@pytest.fixture(scope="module")
def setup():
    rng = random.Random(7)
    log_prob = [[math.log(rng.random()) for _ in ALPHABET] for _ in ALPHABET]
    charsets, normalize = tables(log_prob)
    return StringStats(charsets, normalize, log_prob), charsets, normalize, log_prob


def test_matches_reference(setup):
    stats, charsets, normalize, log_prob = setup
    rng = random.Random(11)
    values = ["", "a", "ab", "abc", "aaaa", "A" * 600, "Kelvin", "tab\tsep", "naïve", "😀x😀"]
    for alphabet in [string.hexdigits, string.ascii_letters + string.digits + "+/=", string.printable]:
        values += ["".join(rng.choice(alphabet) for _ in range(rng.randint(1, 200))) for _ in range(500)]
    got = stats.features_many(values)
    assert len(got) == len(values)
    for s, g in zip(values, got):
        want = reference(s, charsets, normalize, log_prob)
        assert same(want, g), (s, want, g)


def test_unreadable_items_are_false(setup):
    stats = setup[0]
    assert stats.features_many(["lone\ud800", 5, None, b"x", "abc"])[:4] == [False] * 4


def test_rejects_inconsistent_tables():
    with pytest.raises(ValueError):
        StringStats([("ab", [0, 1], [0.5], [0.1, 0.2])], {}, [])
    with pytest.raises(ValueError):
        StringStats([], {97: 3}, [[0.0]])
