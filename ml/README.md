# ml/ — обучение нейро-источников звука

Python-часть нейро-синтеза ASMR X3000: данные → признаки → DDSP-голос + модель песни на вид →
экспорт весов в `core/assets/neural/*.asnw` → инференс в Rust (`core/src/neural/`).
Почему именно так: [docs/neural-sound.md](../docs/neural-sound.md).

## Установка (macOS, Apple Silicon)

```bash
curl -LsSf https://astral.sh/uv/install.sh | sh     # uv в ~/.local/bin
cd ml
uv sync                                              # Python 3.12 + зависимости из uv.lock
uv run python -c "import torch; print(torch.backends.mps.is_available())"   # True
```

Зависимости: PyTorch (MPS), torchaudio, librosa, soundfile (libsndfile: wav/flac/ogg/mp3),
scipy, numpy, torchcrepe (установлен, но для птиц не используется: CREPE не видит выше ~2 кГц),
requests, matplotlib. Rust: `. "$HOME/.cargo/env"` (для `listen` и проверки весов).

Ничего тяжёлого в git не попадает: `ml/data/`, `ml/runs/`, `ml/out/`, `ml/.env` в `.gitignore`.
В git только экспортированные веса (~240 КиБ на вид) в `core/assets/neural/`.

## Ключи API

Положить в `ml/.env` (в git не попадает), по строке `ИМЯ=значение`:

```
XC_API_KEY=...          # https://xeno-canto.org/account
FREESOUND_API_KEY=...   # https://freesound.org/apiv2/apply
```

Переменные окружения с теми же именами имеют приоритет.

## Конвейер

```bash
# 1. Данные (манифест лицензий: data/raw/manifest.jsonl)
uv run python -m asmr_ml.data.commons                 # без ключа: Wikimedia Commons, CC BY/BY-SA/CC0
uv run python -m asmr_ml.data.commons --refresh-meta  # категории → отсев многовидовых записей
uv run python -m asmr_ml.data.xenocanto chaffinch     # нужен XC_API_KEY; NC/ND не качаются без --allow-nc
uv run python -m asmr_ml.data.freesound stream        # нужен FREESOUND_API_KEY; только CC0/CC BY

# 2. Подготовка: ресемплинг 48 кГц, ФВЧ, спектральный гейт, f0/громкость/усилие, слоги, отбор по SNR
uv run python -m asmr_ml.prep chaffinch robin

# 3. Обучение
uv run python -m asmr_ml.train_voice chaffinch --steps 15000   # DDSP-голос (MPS, ~15–30 мин)
uv run python -m asmr_ml.song chaffinch                        # токены слогов + GRU-синтаксис (CPU, ~1 мин)

# 4. Экспорт в движок + тест совпадения Rust/PyTorch
uv run python -m asmr_ml.export chaffinch        # core/assets/neural/chaffinch.asnw (+ parity/)
cargo test --manifest-path ../core/Cargo.toml    # включает neural::voice::tests::matches_pytorch

# 5. Примеры для прослушивания: оригинал / ресинтез / новая генерация из Rust
uv run python -m asmr_ml.listen chaffinch        # out/listen/chaffinch/{1_real,2_resynth,3_generated}.wav
```

Чтобы новый вид попал в движок, добавьте его в `BUILTIN` в `core/src/neural/mod.rs`.

## Модули

| Файл | Что делает |
|---|---|
| `species.py` | виды, диапазоны f0, допустимые лицензии |
| `data/manifest.py` | манифест записей (источник, автор, лицензия, `usable`) |
| `data/commons.py`, `xenocanto.py`, `freesound.py` | загрузчики; NC/ND записываются в манифест, но не качаются |
| `prep.py` | очистка, признаки (трекер доминантной частоты, громкость, «усилие»), сегментация слогов и песен |
| `dsp.py` | синтез (гармоники + банк биквадов), общий с Rust |
| `voice.py`, `train_voice.py` | DDSP-голос и его обучение (MR-STFT) |
| `song.py` | токенизация слогов (k-means по контурам), банк контуров-образцов, GRU над токенами |
| `export.py` | запись `.asnw` (f16-веса + f32-конфиг) и эталона для parity-теста |
| `listen.py`, `plots.py` | примеры для прослушивания и спектрограммы |

## Результаты

См. таблицы в [docs/neural-sound.md §7](../docs/neural-sound.md#7-результаты).
