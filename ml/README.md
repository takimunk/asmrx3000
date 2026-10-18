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
В git только экспортированные веса в `core/assets/neural/`: база 214 КиБ + адаптеры ~1–7 КБ на вид.

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

# 3. Общая база (все виды сразу) — голос и песня
uv run python -m asmr_ml.multi base chaffinch robin song_thrush nightingale --steps 20000   # MPS, ~40 мин
uv run python -m asmr_ml.multisong base chaffinch robin song_thrush nightingale            # CPU, ~3 мин

# 4. Новый вид = адаптер (база заморожена), минуты
uv run python -m asmr_ml.prep blackbird
uv run python -m asmr_ml.multi adapt blackbird --rank 4 --steps 1500   # ~2 мин; --max-recordings 20
uv run python -m asmr_ml.multisong adapt blackbird                      # < 1 мин

# 5. Экспорт в движок + тесты совпадения Rust/PyTorch
uv run python -m asmr_ml.export_multi --adapter blackbird   # birds_base.asnw + <вид>.asna (+ parity/)
cargo test --manifest-path ../core/Cargo.toml

# 6. Качество по Perch 2.0 (отдельное окружение ml/perch_eval с TensorFlow; веса ~390 МБ скачиваются
#    в ~/.cache/kagglehub при первом запуске)
(cd perch_eval && uv sync && uv run python -c "import kagglehub; kagglehub.model_download('google/bird-vocalization-classifier/tensorFlow2/perch_v2_cpu')")
uv run python -m asmr_ml.quality chaffinch   # out/quality/chaffinch/{real,resynth,generated,other} + perch.json
```

Обучение, экспорт и оценка читают только файлы, принятые в `data/clean/<вид>/prep.json`.
Повторный `prep` сначала готовит новую версию во временной папке, затем атомарно заменяет
манифест и удаляет устаревшие `.npz`. Прерванная до замены манифеста подготовка сохраняет
предыдущий набор. Для старой папки без `prep.json` нужно повторить `prep`.

Первая итерация (отдельная модель на вид, банк образцов) осталась в `train_voice.py`, `song.py`,
`export.py`, `listen.py` — для сравнения; движок использует только базу + адаптеры.

Чтобы новый вид был встроен в движок, добавьте его адаптер в `BUILTIN` в `core/src/neural/mod.rs`
(или загрузите во время работы: `neural::load_adapter(bytes)`).

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
| `multi.py` | общий DDSP-голос с эмбеддингом вида; `base` и `adapt` (эмбеддинг + LoRA) |
| `multisong.py` | общий словарь токенов, GRU-синтаксис и CVAE-декодер контуров с эмбеддингом вида |
| `export_multi.py` | `birds_base.asnw` + адаптеры `<вид>.asna` + эталоны parity |
| `quality.py`, `perch_eval/` | оценка Perch 2.0: узнаёт ли вид, близость эмбеддингов к реальным записям |
| `baseline.py` | бейзлайн «синусоида без сети» для шкалы MR-STFT |
| `export.py` | (первая итерация) запись `.asnw` отдельной модели |
| `listen.py`, `plots.py` | примеры для прослушивания и спектрограммы |

## Результаты

См. таблицы в [docs/neural-sound.md §7](../docs/neural-sound.md#7-результаты).
