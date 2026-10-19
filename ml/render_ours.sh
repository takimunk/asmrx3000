#!/usr/bin/env bash
# Renders the current models for comparison with the references (asmr_ml.refcompare).
# Output: ml/out/ours/<category>/<label>.wav     (then: ./render_ours.sh && ./compare_all.sh)
set -euo pipefail
cd "$(dirname "$0")/.."
. "$HOME/.cargo/env"
cargo build --release --manifest-path core/Cargo.toml --example model_wav 2>&1 | tail -1
M=core/target/release/examples/model_wav
O=ml/out/ours
r() { cat=$1; label=$2; shift 2; mkdir -p $O/$cat; $M "$@" $O/$cat/$label.wav 2>/dev/null >/dev/null || true; }
# usage: r <category> <label> <model> <preset> <seconds> <extra...>: out path must come before extras
r2() { cat=$1; label=$2; model=$3; preset=$4; secs=$5; shift 5; mkdir -p $O/$cat; $M "$model" "$preset" "$secs" $O/$cat/$label.wav "$@" 2>&1 | tail -1; }
r2 brook brook_pebbles water brook_pebbles 60
r2 brook brook_rocks water brook_rocks 60
r2 slow_river slow_river water slow_river 60
r2 lake_lapping lake_lapping water lake_lapping 60 wind_ms=3
r2 still_lake_ambience still_lake water still_lake 60 wind_ms=1
r2 waterfall_small small_waterfall water small_waterfall 60
r2 wind_trees forest_roar_5ms wind forest_roar 90 wind_ms=5 gust_amp=0.5
r2 wind_trees breeze_3ms wind breeze 90 wind_ms=3 gust_amp=0.5
r2 wind_trees foliage_oak_5ms foliage oak 90 wind_ms=5 gust_amp=0.5
r2 wind_trees foliage_birch_4ms foliage birch 90 wind_ms=4 gust_amp=0.5
r2 wind_open breeze_4ms wind breeze 90 wind_ms=4 gust_amp=0.3
r2 wind_open foliage_grass_4ms foliage grass 90 wind_ms=4 gust_amp=0.3
r2 wind_open dune_sand_6ms wind dune_sand 90 wind_ms=6 gust_amp=0.3
r2 wind_gusty breeze_6ms_gusty wind breeze 120 wind_ms=6 gust_amp=0.9 gust_period=5
r2 wind_gusty forest_roar_7ms_gusty wind forest_roar 120 wind_ms=7 gust_amp=0.9 gust_period=5
r2 thunder_close close_1500m thunder close_strike 120 distance=1500 interval=20
r2 thunder_close storm_1500m thunder storm 120 distance=1500 interval=20
r2 thunder_distant distant_8000m thunder distant_rumble 120 distance=8000 interval=25
r2 thunder_distant storm_8000m thunder storm 120 distance=8000 interval=25
r2 wings_pigeon wood_pigeon wings:wood_pigeon - 60 flight=7 pause=3
r2 wings_small chaffinch wings:chaffinch - 60 flight=6 pause=3
r2 wings_small robin wings:robin - 60 flight=6 pause=3
r2 wings_small swallow wings:swallow - 60 flight=8 pause=3
r2 wings_large gull wings:gull - 60 flight=10 pause=3
r2 wings_large raven wings:raven - 60 flight=10 pause=3
r2 rain_forest on_leaves rain on_leaves 60 rain_mm_h=4
r2 rain_forest under_canopy rain under_canopy 60 rain_mm_h=4
r2 frogs_marsh marsh_frogs insects marsh_frogs 90 hour=23 temperature=18
r2 frogs_tree tree_frogs insects tree_frogs 90 hour=23 temperature=18
r2 frogs_common common_frogs insects common_frogs 90 hour=23 temperature=18
r2 woodpecker_drum woodpecker insects woodpecker 120 hour=8 temperature=18
