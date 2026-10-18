# ASMR X3000

Физически правдоподобная симуляция звукового окружения в реальном времени: пространственный звук и трекинг головы через AirPods. Подробности в [SPEC.md](SPEC.md).

## Структура

```
core/          Rust: движок (источники → распространение → Ambisonics 3-го порядка → бинаурал)
  include/     C ABI (asmr.h), единственный интерфейс для оболочек
  src/scene.rs, src/director/   граф сцены и режиссёр (docs/director.md)
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

```bash
cargo run --release --manifest-path core/Cargo.toml --example render -- 30 forest.wav turn
```

Демо движения относительно головы (пролёт, стая, подлёт/удаление, круги) с таймлайном событий:

```bash
cargo run --release --manifest-path core/Cargo.toml --example render -- 60 flyby.wav flyby
```

## Заметки
- Если в Центре управления включено системное «Spatialize Stereo» для этого приложения, его надо выключить: спатиализацию делает наш движок, иначе она применится дважды.
- Сборка подписана ad-hoc, поэтому после пересборки macOS может снова спросить разрешение на «Движение и фитнес».
- Если поворот головы ощущается зеркально, поменяйте знак в `HeadTracker.swift` (`yawSign` / `pitchSign` / `rollSign`).
- `ASMRX3000 --snapshot out.png [--select tree|bird|river|none] [--orbit 0..1] [--azimuth рад]` (бинарник внутри бандла) прогоняет движок 11 секунд без аудиоустройства и сохраняет весь поповер (Metal-сцена, спектрограмма, дорожки) в PNG @2x. `ASMR_DEBUG=1` показывает материалы вместо гравюры.
- `ASMRX3000 --bench` открывает поповер в обычном окне на 20 с, потом прячет на 10 с и печатает загрузку CPU процесса в обоих случаях (без клика по menu bar).
- Модели: `/Applications/Blender.app/Contents/MacOS/Blender -b --factory-startup --python tools/blender/build_models.py` (с `PREVIEW=out.png` — ещё и рендер всех моделей рядом). Результат закоммичен, для сборки Blender не нужен.
- Клавиши в сцене: стрелки — двигать выбранное по земле (относительно камеры, по осям мира), ⇧↑/⇧↓ — выше/ниже, ⌥ — шаг 5 см, tab/⇧tab — следующий/предыдущий объект, esc — снять выбор, пробел — play/pause.
- rustup ставился с `--no-modify-path`: чтобы `cargo` был в терминале, добавьте `. "$HOME/.cargo/env"` в `~/.zshrc`. Скрипт сборки находит cargo сам.
