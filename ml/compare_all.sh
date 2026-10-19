#!/usr/bin/env bash
# Compares every rendered model in ml/out/ours/<category>/ with the references of that category.
cd "$(dirname "$0")"
for d in out/ours/*/; do
  c=$(basename "$d")
  [ -d data/refs/$c ] || continue
  ~/.local/bin/uv run python -m asmr_ml.refcompare compare "$c" "$d"*.wav
done
