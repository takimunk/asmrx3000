# ASMR X3000

Физически правдоподобная симуляция звукового окружения в реальном времени: пространственный звук и трекинг головы через AirPods. Подробности в [SPEC.md](SPEC.md).

## Структура

```
core/          Rust: движок (источники → распространение → Ambisonics 3-го порядка → бинаурал)
  include/     C ABI (asmr.h), единственный интерфейс для оболочек
  examples/    render.rs — офлайн-рендер сцены в WAV
apps/macos/    SwiftUI menu bar app (AVAudioEngine + CoreMotion + ASCII-консоль)
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

## Заметки
- Если в Центре управления включено системное «Spatialize Stereo» для этого приложения, его надо выключить: спатиализацию делает наш движок, иначе она применится дважды.
- Сборка подписана ad-hoc, поэтому после пересборки macOS может снова спросить разрешение на «Движение и фитнес».
- Если поворот головы ощущается зеркально, поменяйте знак в `HeadTracker.swift` (`yawSign` / `pitchSign` / `rollSign`).
- `ASMRX3000 --snapshot out.png` (бинарник внутри бандла) прогоняет движок и консоль 6 секунд без аудиоустройства и сохраняет кадр консоли в PNG. Удобно для визуальной проверки UI без клика по иконке.
- rustup ставился с `--no-modify-path`: чтобы `cargo` был в терминале, добавьте `. "$HOME/.cargo/env"` в `~/.zshrc`. Скрипт сборки находит cargo сам.
