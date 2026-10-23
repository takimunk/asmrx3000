# ASMR X3000

Физически правдоподобная симуляция звукового окружения в реальном времени: пространственный звук и трекинг головы через AirPods. Подробности в [SPEC.md](SPEC.md).

## Структура

```
core/          Rust: движок (источники → распространение → Ambisonics 3-го порядка → бинаурал)
  include/     C ABI (asmr.h), единственный интерфейс для оболочек
  src/scene.rs, src/director/   граф сцены и режиссёр (docs/director.md)
  src/direction/  режиссура места: время, погодные эпизоды, коррекция (docs/direction.md)
  scenes/      описания сцен (JSON): лес у реки, демо пространственного звука
  examples/    render.rs — офлайн-рендер сцены в WAV
docs/          director.md — режиссёр, формат сцены, поведения
apps/macos/    SwiftUI menu bar app (AVAudioEngine + CoreMotion + Metal-сцена)
tools/blender/ build_models.py — модели сцены (Blender, без GUI) → apps/macos/Resources/models.bin
scripts/       сборка
```

## Требования
- macOS 14+, Xcode (полный), Rust (rustup)
- AirPods Pro / Max / 3-го поколения и новее, чтобы работал трекинг головы

## Сборка и запуск

```bash
scripts/build-macos.sh
```

Тесты и офлайн-рендер ядра:

```bash
cargo test --manifest-path core/Cargo.toml
```

Регрессионные проверки оболочки (без аудиоустройства), веб-запуска и отбора ML-данных:

```bash
scripts/check-macos.sh
node --test site/tests/*.test.cjs
PYTHONPATH=ml python3 -m unittest discover -s ml/tests
```

Веб-движок после изменений Rust пересобирается отдельно:

```bash
cargo build --release --manifest-path core/Cargo.toml --target wasm32-unknown-unknown
cp core/target/wasm32-unknown-unknown/release/asmr_core.wasm site/asmr_core.wasm
```

```bash
cargo run --release --manifest-path core/Cargo.toml --example render -- 30 forest.wav turn
```

Жизнь птиц (на заданный час): таймлайн занятий каждой птицы — поёт, кормится на земле, летит своим стилем, парит, токует, тревожится — и сводка перекличек (docs/director.md, «Жизнь птиц»):

```bash
cargo run --release --manifest-path core/Cargo.toml --example render -- 180 dawn.wav life:5.5
cargo run --release --manifest-path core/Cargo.toml --example render -- 240 open.wav life:7 core/scenes/open_country.json
```

Режиссура на N часов вперёд без звука: хроника погодных эпизодов, таблица и сводка (docs/direction.md):

```bash
cargo run --release --manifest-path core/Cargo.toml --example direction -- 10 hour=6 cue=150:thunderstorm
```

Демо движения относительно головы (пролёт, стая, подлёт/удаление, круги) с таймлайном событий:

```bash
cargo run --release --manifest-path core/Cargo.toml --example render -- 60 flyby.wav flyby
```

## Заметки
- Если в Центре управления включено системное «Spatialize Stereo» для этого приложения, его надо выключить: спатиализацию делает наш движок, иначе она применится дважды.
- Сборка подписана ad-hoc, поэтому после пересборки macOS может снова спросить разрешение на «Движение и фитнес».
- Если поворот головы ощущается зеркально, поменяйте знак в `HeadTracker.swift` (`yawSign` / `pitchSign` / `rollSign`).
- `ASMRX3000 --snapshot out.png [--select tree|bird|river|none] [--expand water|fire|…|none] [--more] [--spectrogram] [--flight м/с] [--gaze рыск°,тангаж°] [--prompt "…"] [--orbit 0..1] [--azimuth рад] [--set ключ=значение …] [--scene файл.json]` (бинарник внутри бандла) прогоняет движок 11 секунд без аудиоустройства и сохраняет весь поповер (Metal-сцена, спектрограмма, рычаги) в PNG @2x. `--expand` открывает панель группы, `--more` показывает все рычаги, `--spectrogram` раскрывает спектрограмму. `--set` задаёт любой контрол (`--set rain.rate=8 --set env.time_of_day=21.5`), `--scene` загружает JSON-сцену (`apps/macos/Support/snapshot_campfire.json` — лес с костром). `DEBUG_VIEW` в `Shaders.swift` показывает материалы вместо тонального рисунка.
- `ASMRX3000 --bench` открывает поповер в обычном окне на 20 с, потом прячет на 10 с и печатает загрузку CPU процесса в обоих случаях (без клика по menu bar).
- Модели: `/Applications/Blender.app/Contents/MacOS/Blender -b --factory-startup --python tools/blender/build_models.py` (с `PREVIEW=out.png` — ещё и рендер всех моделей рядом). Результат закоммичен, для сборки Blender не нужен.
- Клавиши в сцене: стрелки — двигать выбранное по земле (относительно камеры, по осям мира), ⇧↑/⇧↓ — выше/ниже, ⌥ — шаг 5 см, tab/⇧tab — следующий/предыдущий объект, esc — снять выбор (затем закрыть панель группы), пробел — play/pause, F — в мире: остановиться здесь / лететь дальше, из дома — взлететь (переключатель режима «Дома | Стоим | Летим»), W/S — идти вперёд/назад туда, куда смотрит голова (в полёте — быстрее/медленнее), A/D — шаг влево/вправо (в полёте — поворот), Q/E — повернуть, ⇧ с W/A/S/D — бегом (клавиши по положению, работают и в русской раскладке), H — трекинг головы, C — recenter, ⌥пробел — сказать голосом (fal Whisper, ключ FAL_KEY), return или / — к полю промпта (esc из поля — обратно в сцену), ⌘Z — отменить последний промпт, ⌘← — закрыть панель группы. Интерфейс описан в `docs/ux.md`.
- rustup ставился с `--no-modify-path`: чтобы `cargo` был в терминале, добавьте `. "$HOME/.cargo/env"` в `~/.zshrc`. Скрипт сборки находит cargo сам.
