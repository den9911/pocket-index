# Pocket Index

![Pocket Index illustrated cover](web/cover.svg)

**English** · [Русский](#русский)

A quiet, local search space for Markdown and text notes. Rust and SQLite FTS5 keep your library searchable on your computer.

## Get started

Install [Rust and Cargo](https://rustup.rs/), run `cargo run --release` and open <http://127.0.0.1:3002>. On Windows use `run.bat`; on macOS run `sh run.command`. Paste the absolute path to a folder and select **Build index**. Stop with Ctrl+C.

The index is saved as `pocket-index.sqlite` in the working directory; set `POCKET_INDEX_DB` to choose another database path. Only a fully readable scan replaces the previous index. The app reads `.md` and `.txt` files up to 1 MiB; symbolic links and unsupported extensions are skipped intentionally. If any supported file is unreadable, invalid UTF-8 or too large, the entire new snapshot is discarded and the previous index stays searchable. Fix the problem and rebuild; accepting a partial index is not offered. A complete, readable empty folder publishes an empty index. Only the indexed text stays in the local database; treat it as private. Source files are not modified. Back up or delete the database separately as needed.

Results highlight matching terms. Search uses Unicode-aware exact terms, not semantic search or Russian stemming. Results are ranked by SQLite FTS5; up to 50 are shown. This local server is not intended to be exposed to a network. The cover is an illustration, not a screenshot.

## Русский

Локальный поиск по заметкам Markdown и текстовым файлам. Rust и SQLite FTS5 создают индекс прямо на вашем компьютере.

Установите [Rust и Cargo](https://rustup.rs/), запустите `cargo run --release` и откройте <http://127.0.0.1:3002>. В Windows можно использовать `run.bat`, на macOS — `sh run.command`. Укажите полный путь к папке и нажмите **Построить индекс**. Остановка — Ctrl+C.

Индекс хранится в `pocket-index.sqlite` в рабочей папке; путь можно задать через `POCKET_INDEX_DB`. Прежний индекс заменяется только после полного успешного чтения. Обрабатываются `.md` и `.txt` до 1 МиБ; символические ссылки и другие расширения намеренно пропускаются. Если поддерживаемый файл недоступен, имеет неверную UTF-8 или превышает лимит, новый снимок целиком отклоняется, а поиск продолжает использовать прежний. Исправьте причину и повторите построение; принять неполный индекс пока нельзя. Полный успешный обход пустой папки публикует пустой индекс. Индекс содержит текст заметок, поэтому храните его как личные данные. Исходные файлы не изменяются.

Совпадения подсвечиваются в результатах. Поиск использует точные слова с поддержкой Unicode, без поиска по смыслу и склонениям русского языка. Результаты упорядочиваются SQLite FTS5; отображается до 50. Сервер предназначен для локального использования. Обложка — иллюстрация, не скриншот.

### Search modes / Режимы поиска
Choose All words, Exact phrase or Word prefix. The server builds a bounded literal FTS query (256 characters, 12 terms); arbitrary FTS operators are not accepted. Punctuation separates terms. Prefix matching is not Russian stemming or semantic search.
Режимы: все слова, точная фраза, начало слова. Знаки пунктуации разделяют слова; до 256 символов и 12 слов. Это не поиск по смыслу или склонениям.
Diagnostics: `cargo run --release -- --check` checks the database location and local port without indexing or reading notes. / Проверка запуска без чтения заметок: `--check`.

### Safe rebuild / Безопасное перестроение
The status separates files prepared in the current scan from documents in the active snapshot. Its persisted generation changes only on a successful commit. Search responses keep the existing JSON array and add `x-pocket-index-generation`; the interface warns if displayed results came from an older generation. Database migration adds only snapshot metadata; existing FTS text stays intact. Reads are bounded to 1 MiB plus one sentinel byte, even if a file grows. Errors expose fixed reason codes, not paths or note text.

Счётчик подготовленных файлов не равен размеру действующего индекса. Версия снимка сохраняется в SQLite и меняется только после commit. Если результаты относятся к старой версии, интерфейс предлагает повторить поиск, сохраняя запрос. Миграция добавляет только метаданные снимка. Фактическое чтение ограничено 1 МиБ и одним проверочным байтом даже при росте файла. Диагностика не раскрывает пути и текст заметок.

Rebuilds allow at most 100,000 tree entries and a cooperative five-minute budget checked between operations and after reads. A filesystem call that stalls cannot be forcibly interrupted by this budget; select a normal local folder. Limit exhaustion retains the old snapshot. / Обход ограничен 100 000 записями дерева и проверяемым между операциями лимитом пяти минут; зависший системный вызов этим лимитом принудительно не прерывается. Превышение сохраняет прежний снимок.
