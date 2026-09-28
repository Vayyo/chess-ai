# ♟️ chess-ai

> Шахматный движок на Rust: легальные ходы на битбордах, поиск с NNUE-оценкой и игра через UCI или локальный браузер.

![Rust edition 2024](https://img.shields.io/badge/Rust-edition%202024-orange)
![UCI](https://img.shields.io/badge/protocol-UCI-blue)

---

## ✨ Возможности

| Возможность | Как пользоваться |
|---|---|
| ♟️ UCI | Подключить `chess-ai` к шахматному GUI; есть опция `Hash`, команды `go perft <N>` и `d`. |
| 🌐 Игра в браузере | Запустить `serve`: уровни от «полная сила» до случайных ходов, отмена хода и подсказка. Остальные уровни, кроме «полной силы», — условные пресеты, а не рейтинги Elo. |
| 🧠 NNUE | Квантованная сеть из `nets/current.bin` встраивается при сборке; для игры не нужны отдельные файлы данных. |
| 🧪 Эксперименты | `datagen` для самоигры, отдельный CUDA-тренер и скрипты SPRT/калибровки; подготовка внешних инструментов нужна только для этих задач. |

---

## 🚀 Быстрый старт

Нужен Rust/Cargo с поддержкой edition 2024. Все команды ниже запускаются из корня
проекта; веса `nets/current.bin` нужны при сборке и входят в исходный код.

```sh
cargo build --release
./target/release/chess-ai
```

По умолчанию запускается UCI. Введите в открытый процесс по одной строке:

```text
uci
isready
position startpos
go depth 4
quit
```

Дождитесь `bestmove` перед `quit`. В UCI-клиенте укажите путь к
`target/release/chess-ai`. Для проверки ходов есть `go perft <N>` (разбивка по
ходам) и `d` (доска, FEN, хеш). `./target/release/chess-ai bench [depth]`
измеряет поиск по фиксированным позициям.

**Игра в браузере** (запустите отдельно):

```sh
./target/release/chess-ai serve 8099
```

Откройте <http://127.0.0.1:8099/>. Без аргумента порт тоже 8099; сервер
слушает только loopback. Страница встроена в бинарник (`crates/engine/assets/`),
браузеру не нужны внешние ресурсы. Есть отмена хода и подсказка.

**Переносимость сборки:** `.cargo/config.toml` включает `target-cpu=native`;
такой бинарник может не запуститься на другом CPU. Для другой машины подберите
подходящий `RUSTFLAGS="-C target-cpu=..."`. При медленном PEXT можно собирать
с `MOVEGEN_NO_PEXT=1` (magic bitboards).

## 🧩 Устройство движка

| Компонент | Роль |
|---|---|
| `crates/movegen` | `Board` (copy-make), FEN, Zobrist, perft и легальные ходы с check/pin-масками. Для дальнобойных фигур — PEXT при BMI2 на x86_64, иначе fancy magic bitboards; таблицы строит `build.rs`. |
| `crates/engine` | Бинарник `chess-ai`: итеративное углубление, PVS, таблица транспозиций (`Hash`), null move, LMR, quiescence, порядок ходов (TT, MVV-LVA, killers, history), продление при шахе, повторения и правило 50 ходов. |
| `crates/engine/src/nnue.rs` | Два аккумулятора перспектив с инкрементальным обновлением; веса встроены из `nets/current.bin` через `include_bytes!`. |
| `trainer` | Отдельный от workspace крейт для обучения через [bullet](https://github.com/jw1912/bullet); требует CUDA. |

---

## 🧠 Сеть и данные

Квантованные веса `nets/current.bin` необходимы для сборки. Данные самоигры
(`data/`), сторонние инструменты и книга дебютов (`tools/`), снимки обучения
(`trainer/checkpoints/`) **не поставляются** и не нужны для игры. Лицензия
проекта не указана.

### Генерация позиций

Пример длительного запуска самоигры (параметры можно уменьшить):

```sh
mkdir -p data
./target/release/chess-ai datagen data/gen0.bin --threads 5 --positions 100000000 --nodes 5000 --seed 1
```

Самоигра: 8–9 случайных полуходов (несбалансированные дебюты отбрасываются), затем
поиск на `--nodes` узлов за ход. Записываются тихие позиции не под шахом с оценкой
поиска и итогом партии; адъюдикация побед (±2000 cp 4 полухода) и ничьих (±10 cp 8
полуходов после 60-го). Формат — `ChessBoard` bullet (32 байта); файл дописывается,
оборванная запись в конце отрезается при следующем запуске.

Для проверки и перемешивания потребуется отдельно полученный [bullet](https://github.com/jw1912/bullet)
с `bullet-utils` (`cargo build -r -p bullet-utils` в его каталоге); команды ниже
предполагают, что `bullet-utils` доступен в `PATH`:

```sh
bullet-utils validate -i data/gen0.bin
bullet-utils shuffle -i data/gen0.bin -o data/gen0-shuffled.bin -m 4096
```

### Обучение NNUE

```sh
cd trainer
cargo build --release # отдельно от workspace; требуется CUDA
./target/release/trainer my-net ../data/gen0.bin --epochs 20 --wdl 0.3 --lr 0.001
cd ..
```

Архитектура `(768 → 256)×2 → 1` SCReLU, квантование QA=255 / QB=64. Цель —
`wdl * итог партии + (1 - wdl) * sigmoid(оценка поиска / 400)`.
Одна эпоха — один проход по данным; `--epochs` задаёт горизонт расписания.
Результат — `trainer/checkpoints/my-net-<эпоха>/quantised.bin`: для использования
скопируйте выбранные веса в `nets/current.bin` и пересоберите движок.
В одном локальном замере на GTX 1660 SUPER обучение шло около 6 млн позиций/с;
это не гарантированная скорость. После замены сети проверьте
`cargo test --workspace`, `./target/release/chess-ai bench`, затем SPRT.

---

## 📊 Измерение силы

**SPRT** сравнивает две версии между собой, а не присваивает абсолютный рейтинг.
История экспериментов — [журнал SPRT](docs/sprt-log.md).
Нужны отдельно установленные [fastchess](https://github.com/Disservin/fastchess)
и [книга дебютов Stockfish](https://github.com/official-stockfish/books):
скрипт ожидает `tools/fastchess/fastchess` и `tools/8moves_v3.pgn`.
Разовая подготовка из корня проекта:

```sh
mkdir -p tools
git clone --depth 1 https://github.com/Disservin/fastchess.git tools/fastchess
make -C tools/fastchess -j
curl -L https://github.com/official-stockfish/books/raw/master/8moves_v3.pgn.zip -o tools/8moves_v3.pgn.zip
(cd tools && unzip 8moves_v3.pgn.zip)
```

`scripts/sprt.sh <base-rev> [new-rev]` сравнивает ревизии; без второго аргумента
использует текущее рабочее дерево. Пример команды:

```sh
TC=10+0.1 CONCURRENCY=5 ELO0=0 ELO1=5 scripts/sprt.sh HEAD~1 HEAD
```

`scripts/build-rev.sh <rev>` собирает ревизию в `tools/bin/chess-ai-<sha>`;
для режимов по узлам есть `NODES=50000` (они не показывают изменение скорости).
Для осмысленного сравнения учитывайте одинаковые сборку, книгу и контроль времени.

**Калибровка с внешним движком.** В [журнале калибровки](docs/calibration-log.md)
записан локальный результат порядка **3000** по шкале CCRL-блица в конкретных
матчах с Princhess (например, ≈3010 против 0.14.1 на 50 партиях).
Это не универсальный или официальный рейтинг chess-ai: контроль и железо
отличаются от CCRL, выборка мала. Пример воспроизведения из корня проекта
(после подготовки fastchess и книги выше; рейтинг соперника сверяйте с
[CCRL Blitz](https://computerchess.org.uk/ccrl/404/)):

```sh
git clone --depth 1 -b 0.14.1 https://github.com/princesslana/princhess.git tools/princhess
(cd tools/princhess && RUSTFLAGS="-C target-cpu=native" cargo build --release)
OPPONENT_NAME=princhess-0.14.1 CCRL=2947 scripts/calibrate.sh \
  "$PWD/tools/princhess/target/release/princhess" 50 tc=60+0.6
```

Показатель CCRL=2947 для Princhess 0.14.1 взят на 2026-09-27. Оценка равна
рейтингу соперника плюс `-400 * log10(1 / доля_очков - 1)`; при 50% поправка
нулевая. Скрипт сохраняет PGN и файлы `.rating`/`.opponent` для
`scripts/dashboard.sh`.

При интерпретации матчей:

- `UCI_LimitStrength`/`UCI_Elo` у Stockfish выбирают ослабленные ходы, это не
  опубликованный рейтинг соперника (в локальном сравнении отметки 2000 и 2600
  давали несогласующиеся оценки ≈2390 и ≈2775).
- Проверяйте `Termination` в PGN: старые движки могут проигрывать по времени,
  не учитывая приращение (Stockfish 2.0.1: 15 просрочек из 38 на `10+0.1`).
  Для старых UCI-движков, которым мешает конвейер команд fastchess, есть
  `scripts/uci-serialize.py`.
- Соперника тоже собирайте с `RUSTFLAGS="-C target-cpu=native"`: сравнение с
  базовым x86-64 может дать ложное преимущество в сотни Elo.

Внешние матчи через `lichess-bot` могут дать рейтинг Glicko в пуле Lichess,
но он измеряется по другой шкале.
