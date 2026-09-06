#!/usr/bin/env python3

import csv
import io
import json
import subprocess
import sys


def main(argv):
    if len(argv) in (3, 4) and argv[1] == "--image":
        return recognize_image(argv[2], argv[3] if len(argv) == 4 else None)
    if len(argv) != 3:
        print("usage: slice-text-finder.py QUERY TESSERACT_TSV", file=sys.stderr)
        return 2
    query = " ".join(argv[1].casefold().split())
    if not query:
        print("query must not be empty", file=sys.stderr)
        return 2

    matches = find_matches(query, read_lines(argv[2]))
    if not matches:
        print("null")
        return 1
    for match in matches:
        print(json.dumps(match, ensure_ascii=False, separators=(",", ":")))
    return 0


def read_lines(path):
    with open(path, newline="", encoding="utf-8") as handle:
        return parse_lines(handle)


def parse_lines(handle):
    lines = {}
    for row in csv.DictReader(handle, delimiter="\t"):
        if not (row.get("text") or "").strip():
            continue
        key = tuple(int(row.get(field) or 0)
                    for field in ("page_num", "block_num", "par_num", "line_num"))
        lines.setdefault(key, []).append(row)
    return [
        sorted(rows, key=lambda row: int(row.get("word_num") or 0))
        for _, rows in sorted(lines.items())
    ]


def recognize_image(image, query):
    normalized = " ".join(query.casefold().split()) if query is not None else None
    if normalized == "":
        raise ValueError("query must not be empty")
    text_lines, matches = [], []
    # Automatic page segmentation misses small windows on a dark desktop.
    # Keep its layout coverage and add a block pass, in original pixel space.
    # In-memory TSV also avoids concurrent reads sharing one temporary file.
    for mode in (3, 6):
        result = subprocess.run(
            ["tesseract", image, "stdout", "-l", "eng", "--psm", str(mode), "tsv"],
            check=True, capture_output=True, text=True, encoding="utf-8", timeout=15,
        )
        lines = parse_lines(io.StringIO(result.stdout))
        if normalized is None:
            for words in lines:
                line = " ".join(row["text"].strip() for row in words)
                if line not in text_lines:
                    text_lines.append(line)
        else:
            for match in find_matches(normalized, lines):
                if not any(same_target(match, previous) for previous in matches):
                    matches.append(match)
    if normalized is None:
        print("\n".join(text_lines))
        return 0
    if not matches:
        print("null")
        return 1
    for match in sorted(matches, key=lambda value: (value["top"], value["left"])):
        print(json.dumps(match, ensure_ascii=False, separators=(",", ":")))
    return 0


def same_target(a, b):
    width = max(0, min(a["left"] + a["width"], b["left"] + b["width"]) - max(a["left"], b["left"]))
    height = max(0, min(a["top"] + a["height"], b["top"] + b["height"]) - max(a["top"], b["top"]))
    smaller = min(a["width"] * a["height"], b["width"] * b["height"])
    return smaller > 0 and width * height >= smaller * 0.5


def find_matches(query, lines):
    matches = []
    seen_boxes = set()
    for words in lines:
        normalized_words = [
            " ".join((row.get("text") or "").strip().casefold().split())
            for row in words
        ]
        searchable = " ".join(normalized_words)
        word_spans = []
        offset = 0
        for word in normalized_words:
            word_spans.append((offset, offset + len(word)))
            offset += len(word) + 1

        cursor = 0
        while (found := searchable.find(query, cursor)) >= 0:
            found_end = found + len(query)
            selected = [
                row
                for row, (start, end) in zip(words, word_spans)
                if end > found and start < found_end
            ]
            if selected:
                match = match_for_rows(selected)
                box = tuple(match[field] for field in ("left", "top", "width", "height"))
                if box not in seen_boxes:
                    matches.append(match)
                    seen_boxes.add(box)
            cursor = found_end
    return matches


def match_for_rows(rows):
    left = min(int(row["left"]) for row in rows)
    top = min(int(row["top"]) for row in rows)
    right = max(int(row["left"]) + int(row["width"]) for row in rows)
    bottom = max(int(row["top"]) + int(row["height"]) for row in rows)
    return {
        "text": " ".join((row.get("text") or "").strip() for row in rows),
        "left": left,
        "top": top,
        "width": right - left,
        "height": bottom - top,
        "center_x": (left + right) // 2,
        "center_y": (top + bottom) // 2,
    }


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv))
    except (OSError, TypeError, ValueError, subprocess.SubprocessError) as error:
        print(f"slice text lookup failed: {error}", file=sys.stderr)
        sys.exit(2)
