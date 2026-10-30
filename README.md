# Pocket Index

![Pocket Index illustrated cover](web/cover.svg)

**English** · [Русский](#русский)

A quiet, local search space for Markdown and text notes. Rust and SQLite FTS5 keep your library searchable on your computer.

## Get started

Install [Rust and Cargo](https://rustup.rs/), run `cargo run --release` and open <http://127.0.0.1:3002>. On Windows use `run.bat`; on macOS run `sh run.command`. Paste the absolute path to a folder and select **Build index**. Stop with Ctrl+C.

The index is saved as `pocket-index.sqlite` in the working directory; set `POCKET_INDEX_DB` to choose another database path. Rebuilding replaces the previous index. The app reads `.md` and `.txt` files up to 1 MiB and skips symbolic links, unsupported files, invalid UTF-8, and unreadable files. Only the indexed text stays in the local database; treat it as private. Source files are not modified. Back up or delete the database separately as needed.

Results highlight matching terms. Search uses Unicode-aware exact terms, not semantic search or Russian stemming. Results are ranked by SQLite FTS5; up to 50 are shown. This local server is not intended to be exposed to a network. The cover is an illustration, not a screenshot.

## Русский

Локальный поиск по заметкам Markdown и текстовым файлам. Rust и SQLite FTS5 создают индекс прямо на вашем компьютере.

Установите [Rust и Cargo](https://rustup.rs/), запустите `cargo run --release` и откройте <http://127.0.0.1:3002>. В Windows можно использовать `run.bat`, на macOS — `sh run.command`. Укажите полный путь к папке и нажмите **Построить индекс**. Остановка — Ctrl+C.

Индекс хранится в `pocket-index.sqlite` в рабочей папке; путь можно задать через `POCKET_INDEX_DB`. Повторное построение заменяет предыдущий индекс. Обрабатываются файлы `.md` и `.txt` до 1 МиБ; символические ссылки, другие файлы, неверная кодировка UTF-8 и недоступные файлы пропускаются. Индекс содержит текст заметок, поэтому храните его как личные данные. Исходные файлы не изменяются.

Совпадения подсвечиваются в результатах. Поиск использует точные слова с поддержкой Unicode, без поиска по смыслу и склонениям русского языка. Результаты упорядочиваются SQLite FTS5; отображается до 50. Сервер предназначен для локального использования. Обложка — иллюстрация, не скриншот.
