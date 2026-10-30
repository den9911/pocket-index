# Project notes

The localhost Axum server accepts an explicit folder path and runs a blocking scanner on a worker thread. WalkDir does not follow symbolic links. SQLite FTS5 persists paths, names and text, ranks results and uses the Unicode tokenizer. The previous index is replaced inside one transaction; a failed build rolls back the replacement. The status endpoint reports progress and skipped files. Search results are inserted into the DOM as text, not HTML; matching terms are marked without interpreting document content as markup.

Limitations: one active index and one build at a time; no stemming, PDF, OCR or file watcher. The local database contains copies of note text. Future work: real screenshots, richer error reporting, index selection and Unicode language-specific analysis.
