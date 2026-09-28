# chess-ai

Шахматный движок на Rust с генерацией ходов на битбордах, поиском и NNUE-оценкой,
обученной на самоигре. Бинарник работает как UCI-движок или локальный веб-сервер.

## Структура

| Крейт | Что внутри |
|---|---|
| `crates/movegen` | Битборды, легальная генерация ходов (check/pin-маски), copy-make `Board`, Zobrist, FEN, perft. Атаки дальнобойных фигур: PEXT при BMI2 (x86_64), иначе fancy magic bitboards; таблицы генерирует `build.rs`. |
| `crates/engine` | UCI-движок, бинарник `chess-ai`: итеративное углубление, PVS, таблица транспозиций (опция `Hash`), null move pruning, LMR, quiescence, сортировка (TT-ход, MVV-LVA, killers, history), продление при шахе, повторения и правило 50 ходов. Оценка — NNUE: два аккумулятора перспектив, инкрементальное обновление при ходе (`crates/engine/src/nnue.rs`). |
| `trainer` | Обучение сети в [bullet](https://github.com/jw1912/bullet) (нужна CUDA, поэтому крейт отдельный и в workspace не входит). |

Текущая сеть — `nets/current.bin`, встроена в бинарник при сборке.

## Быстрый старт

Нужен Rust с поддержкой edition 2024 (Cargo), а для браузерного режима — браузер.
Движок использует встроенный при сборке `nets/current.bin`; отдельно загружать сеть
для игры не нужно. Из корня проекта:

```sh
cargo build --release
cargo test --workspace
./target/release/chess-ai
```

По умолчанию запускается UCI. Введите в запущенный процесс команды по одной строке:

```text
uci
isready
position startpos
go depth 4
quit
```

Дождитесь `bestmove` перед `quit`. Для UCI-клиента укажите путь к
`target/release/chess-ai`. Дополнительные команды: `go perft <N>` (разбивка по
ходам) и `d` (доска, FEN и хеш). `./target/release/chess-ai bench [depth]`
измеряет поиск по фиксированным позициям.

Локальная игра в браузере:

```sh
./target/release/chess-ai serve 8099
```

Откройте `http://127.0.0.1:8099/`. Без номера порта также используется 8099;
сервер слушает только loopback. Страница встроена в бинарник
(`crates/engine/assets/`). Есть уровни «полная сила», «сложно», «средне»,
«легко», «новичок» и случайные ходы; можно отменить ход и запросить подсказку.

`.cargo/config.toml` собирает под процессор текущей машины (`target-cpu=native`):
такой бинарник может не работать на другом CPU. Для сборки под другую машину
подберите соответствующий `RUSTFLAGS="-C target-cpu=..."`; на процессорах с
медленным PEXT можно задать `MOVEGEN_NO_PEXT=1` для magic bitboards.

## Сеть и данные

`nets/current.bin` — квантованные веса NNUE, необходимые для сборки и встроенные
через `include_bytes!`. Данные самоигры (`data/`), сторонние инструменты и
книги дебютов (`tools/`), снимки обучения (`trainer/checkpoints/`) не нужны для
игры и не входят в распространяемый исходный код. Лицензия проекта не указана.

## Данные для обучения

```sh
./target/release/chess-ai datagen data/gen0.bin --threads 5 --positions 100000000 --nodes 5000 --seed 1
```

Самоигра: 8–9 случайных полуходов (несбалансированные дебюты отбрасываются), затем
поиск на `--nodes` узлов за ход. Записываются тихие позиции не под шахом с оценкой
поиска и итогом партии; адъюдикация побед (±2000 cp 4 полухода) и ничьих (±10 cp 8
полуходов после 60-го). Формат — `ChessBoard` bullet (32 байта); файл дописывается,
оборванная запись в конце отрезается при следующем запуске.

Проверка и перемешивание — утилитами bullet (`cargo build -r -p bullet-utils` в `tools/bullet`):

```sh
bullet-utils validate -i data/gen0.bin
bullet-utils shuffle -i data/gen0.bin -o data/gen0-shuffled.bin -m 4096
```

## Обучение сети

```sh
cd trainer && cargo build --release          # отдельно от workspace; требуется CUDA
./target/release/trainer <net-id> ../data/gen0.bin --epochs 20 --wdl 0.3 --lr 0.001
```

Архитектура `(768 → 256)x2 → 1` SCReLU, квантование QA=255 / QB=64. Цель —
`wdl * итог партии + (1 - wdl) * sigmoid(оценка поиска / 400)`. Одна эпоха — один
проход по данным; `--epochs` и есть horizon расписания обучения. Результат:
`trainer/checkpoints/<net-id>-<эпоха>/quantised.bin` — его и нужно положить
в `nets/current.bin`. В одном локальном замере на GTX 1660 SUPER обучение шло
~6 млн позиций/с.

Проверка сети после подмены: `cargo test --workspace`, `chess-ai bench`, затем SPRT (см. ниже).

## Замеры силы (SPRT)

Разовая подготовка (всё в `tools/`, в git не попадает):

```sh
mkdir -p tools && cd tools
git clone --depth 1 https://github.com/Disservin/fastchess.git && make -C fastchess -j
curl -LO https://github.com/official-stockfish/books/raw/master/8moves_v3.pgn.zip && unzip 8moves_v3.pgn.zip
```

Проверка изменения против базовой ревизии:

```sh
scripts/sprt.sh <base-rev> [new-rev]      # без new-rev — текущее рабочее дерево
TC=10+0.1 CONCURRENCY=5 ELO0=0 ELO1=5 scripts/sprt.sh HEAD~1 HEAD
```

`scripts/build-rev.sh <rev>` собирает любую ревизию в `tools/bin/chess-ai-<sha>`.

## Абсолютная сила

Матчи против себя дают только относительные числа. Правильный якорь — движок с
опубликованным рейтингом (CCRL), играющий в полную силу.

Удобнее всего брать несколько версий одного движка: у них плотная лестница
рейтингов, и точку 50% видно с шагом ~100 Elo. Пример с Princhess
(блиц-рейтинги CCRL на 2026-09-27: 0.12.0 = 2724, 0.13.0 = 2833, 0.14.1 = 2947,
0.15.1 = 3040, 0.16.0 = 3091, 0.18.0 = 3184, 0.22.0 = 3253):

```sh
git clone --depth 1 -b 0.14.1 https://github.com/princesslana/princhess.git /tmp/prin
cd /tmp/prin && RUSTFLAGS="-C target-cpu=native" cargo build --release

OPPONENT_NAME=princhess-0.14.1 CCRL=2947 scripts/calibrate.sh \
    /tmp/prin/target/release/princhess 50 tc=60+0.6
```

Своя оценка = рейтинг соперника плюс поправка за счёт матча
(`-400 * log10(1 / доля_очков - 1)`); при счёте 50% она равна рейтингу соперника.
Матч пишет рядом с PGN файлы `.rating` (рейтинг соперника) и `.opponent`, по ним
`scripts/dashboard.sh` подписывает матчи.

### Четыре ловушки

- **`UCI_LimitStrength` + `UCI_Elo` у Stockfish — не рейтинг.** Движок ищет с полной
  силой, а затем намеренно выбирает не лучший ход из нескольких линий
  (`src/search.cpp`: `Skill::pick_best`). Это подмешивание зевков, и шкала сжатая:
  две точки (2000 и 2600) дают для нас разные оценки — ≈2390 и ≈2775.
- **Проверяйте, что матч не выигран на флажке.** Смотрите `Termination` в PGN:
  должно быть в основном `normal`, а не `time forfeit`. Старые движки на быстрых
  контролях не учитывают приращение и просрочивают (у Stockfish 2.0.1 на `10+0.1`
  было 15 просрочек из 38 партий, глубина 6-7 вместо 18).
- **Движки 2000-х не понимают конвейер команд.** fastchess шлёт `isready` и следующий
  `position`, пока идёт поиск; Stockfish 1.x/2.x от этого не отвечает ходом вовремя.
  Обёртка `scripts/uci-serialize.py` придерживает `position`/`go` до `bestmove`.
- **Собирайте соперника с той же оптимизацией, что и свой движок.** По умолчанию
  Rust целится в базовый x86-64 (только SSE2): против такого бинарника легко набрать
  лишние 200-400 «Elo». Нужен `RUSTFLAGS="-C target-cpu=native"`.

Результаты калибровки — [docs/calibration-log.md](docs/calibration-log.md) (оценка ≈3010
в условиях зафиксированных матчей, не универсальный рейтинг).

Для сверки с внешним пулом можно поднять движок ботом на Lichess (`lichess-bot`,
токен бот-аккаунта) — это даст рейтинг Glicko из реальных партий, но по своей шкале.

## План

1. ~~Генерация ходов, UCI, базовый поиск~~
2. ~~Таблица транспозиций, killer/history, null move, LMR; SPRT-инфраструктура~~ (результаты — [docs/sprt-log.md](docs/sprt-log.md))
3. ~~Генератор данных самоигры (многопоточный, формат bullet)~~
4. ~~Обучение NNUE в [bullet](https://github.com/jw1912/bullet)~~
5. ~~Инференс NNUE: аккумулятор, инкрементальные обновления~~ (SIMD/AVX2 — отдельный шаг: сейчас 3,5 млн узлов/с против 6,1 на PST)
6. Цикл: данные → обучение → SPRT → следующее поколение (идёт: gen1 пишется сетью gen0)
7. Оценка: королевские корзины (`ChessBucketsMirrored`) в bullet + бакетные аккумуляторы в движке
