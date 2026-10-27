"""Chip tags: the EroScripts / FapTap post tags worth showing on a video.

The desktop registers every linked post's tags (`scriptTags`, videoPath →
[tag, ...]) unfiltered. The viewers (web remote chips and Tags view,
HereSphere / DeoVR `Tag:` scene tags) show this filtered list instead:
noise is dropped — length buckets (`len-*`), device tags, `vr` / `non-vr`
(VR comes from the backend's own VR decision), `no-free-links` and
`straight`. Tags are trimmed, lower-cased, de-duplicated and sorted, so the
same tag reads the same on every video and counts add up across the library.
"""

LENGTH_TAG_PREFIX = 'len-'

NOISE_TAGS = frozenset({
    # Devices
    'the-handy', 'the-handy-2-pro', 'osr2', 'sr6', 'funsr-pro', 'syncbot',
    'multi-axis', 'twist',
    # Shown from the VR decision instead
    'vr', 'non-vr',
    # Forum bookkeeping
    'no-free-links', 'straight',
})


def chip_tags(tags):
    """The display list for one video's raw post tags (any iterable; non-
    string entries are skipped)."""
    out = set()
    for tag in tags or ():
        if not isinstance(tag, str):
            continue
        key = tag.strip().casefold()
        if not key or key in NOISE_TAGS or key.startswith(LENGTH_TAG_PREFIX):
            continue
        out.add(key)
    return sorted(out)
