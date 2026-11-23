-- Sage index schema, version 7 (2: links include Markdown links and wikilinks
-- in frontmatter properties; 3: v_tags, v_properties, v_notes.name and .modified;
-- 4: files.cloud, texts, v_notes.cloud; 5: files.id_key, v_documents; 6: v_tags.line;
-- 7: escaped pipes in table links, and code fences found as Obsidian finds them;
-- 8: files.status_key, which v_documents.status reads; 9: tags_file covers line).
--
-- The index is a cache. Everything here is derived from the vault and can be
-- rebuilt from scratch, so a version bump drops and recreates the database
-- instead of migrating it. Nothing user-authored may ever live only in here.

CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;

-- One row per file in the vault, notes and attachments alike. Attachments are
-- tracked so ![[image.png]] resolves, but only .md files are parsed.
CREATE TABLE files (
    id         INTEGER PRIMARY KEY,
    path       TEXT    NOT NULL UNIQUE,  -- vault-relative, '/' separators
    link_key   TEXT    NOT NULL,         -- lowercase path, '.md' stripped: what [[folder/Note]] matches
    name_key   TEXT    NOT NULL,         -- last segment of link_key: what [[Note]] matches
    parent     TEXT    NOT NULL,         -- containing folder, '' for the vault root
    ext        TEXT    NOT NULL,         -- lowercase, no dot
    title      TEXT    NOT NULL,         -- frontmatter title, else first H1, else file name
    mtime_ns   INTEGER NOT NULL,         -- 0 means "racy, recheck by hash on the next pass"
    size       INTEGER NOT NULL,
    hash       BLOB,                     -- blake3 of the content, NULL for attachments
    indexed_at INTEGER NOT NULL,         -- unix seconds
    cloud      INTEGER NOT NULL DEFAULT 0, -- a note that is a OneDrive placeholder, content in the cloud
    id_key     TEXT,                     -- lowercase frontmatter `id`: what [[id]] matches when no file name does
    status_key TEXT                      -- frontmatter `status` as a slug, `_` read as `-`: see v_documents
);
CREATE INDEX files_name_key ON files (name_key);
CREATE INDEX files_id_key   ON files (id_key) WHERE id_key IS NOT NULL;
CREATE INDEX files_parent   ON files (parent);
CREATE INDEX files_hash     ON files (hash) WHERE hash IS NOT NULL;

-- Flattened YAML frontmatter: one row per scalar. Lists repeat the key with
-- increasing ord, nested maps use dotted keys.
CREATE TABLE frontmatter (
    file_id    INTEGER NOT NULL REFERENCES files (id) ON DELETE CASCADE,
    key        TEXT    NOT NULL,         -- lowercase
    ord        INTEGER NOT NULL,
    value_type TEXT    NOT NULL,         -- 'text' | 'num' | 'bool' | 'null'
    value      TEXT,                     -- textual form, always set unless null
    value_num  REAL,                     -- set for 'num' and 'bool', for range queries
    PRIMARY KEY (file_id, key, ord)
) WITHOUT ROWID;
CREATE INDEX frontmatter_key_value ON frontmatter (key, value);
CREATE INDEX frontmatter_key_num   ON frontmatter (key, value_num) WHERE value_num IS NOT NULL;

CREATE TABLE aliases (
    file_id  INTEGER NOT NULL REFERENCES files (id) ON DELETE CASCADE,
    alias    TEXT    NOT NULL,
    alias_lc TEXT    NOT NULL
);
CREATE INDEX aliases_file ON aliases (file_id);
CREATE INDEX aliases_lc   ON aliases (alias_lc);

CREATE TABLE headings (
    file_id INTEGER NOT NULL REFERENCES files (id) ON DELETE CASCADE,
    line    INTEGER NOT NULL,
    level   INTEGER NOT NULL,
    text    TEXT    NOT NULL,
    PRIMARY KEY (file_id, line)
) WITHOUT ROWID;

-- Tags are stored lowercase without '#'. A nested tag query for project/alpha is
--   tag = 'project/alpha' OR tag GLOB 'project/alpha/*'
-- and both forms use tags_tag.
CREATE TABLE tags (
    file_id INTEGER NOT NULL REFERENCES files (id) ON DELETE CASCADE,
    tag     TEXT    NOT NULL,
    line    INTEGER NOT NULL             -- 0 for frontmatter tags
);
CREATE INDEX tags_tag  ON tags (tag, file_id);
CREATE INDEX tags_file ON tags (file_id, line); -- a note's tags, and a task line's

-- Forward links are rows by src_file_id, backlinks are rows by dst_file_id.
-- A target matches a file by name, else a note by its frontmatter `id`.
-- dst_file_id is NULL while the target does not exist; the indexer fills it in
-- when a matching file appears and SQLite clears it when the target is deleted.
-- Wikilinks and Markdown links share the table.
CREATE TABLE links (
    id          INTEGER PRIMARY KEY,
    src_file_id INTEGER NOT NULL REFERENCES files (id) ON DELETE CASCADE,
    dst_file_id INTEGER          REFERENCES files (id) ON DELETE SET NULL,
    target      TEXT    NOT NULL,        -- as written, Markdown targets percent-decoded
    target_key  TEXT    NOT NULL,        -- lowercase, '.md' stripped, ./ and ../ resolved
    target_name TEXT    NOT NULL,        -- last path segment of target_key
    heading     TEXT,
    block       TEXT,
    alias       TEXT,
    is_embed    INTEGER NOT NULL DEFAULT 0,
    line        INTEGER NOT NULL,
    col         INTEGER NOT NULL,        -- byte offset in the line
    len         INTEGER NOT NULL
);
CREATE INDEX links_src        ON links (src_file_id);
CREATE INDEX links_dst        ON links (dst_file_id) WHERE dst_file_id IS NOT NULL;
CREATE INDEX links_unresolved ON links (target_name) WHERE dst_file_id IS NULL;
CREATE INDEX links_name       ON links (target_name);

CREATE TABLE tasks (
    id        INTEGER PRIMARY KEY,
    file_id   INTEGER NOT NULL REFERENCES files (id) ON DELETE CASCADE,
    line      INTEGER NOT NULL,
    indent    INTEGER NOT NULL,
    state     TEXT    NOT NULL,          -- the checkbox character: ' ', 'x', ...
    text      TEXT    NOT NULL,          -- after the checkbox, metadata included
    raw       TEXT    NOT NULL,          -- full source line, verified before write-back
    status    TEXT,                      -- slug from #status/<slug>
    due       TEXT,                      -- ISO dates sort correctly as text
    scheduled TEXT,
    start     TEXT,
    done_on   TEXT,
    priority  INTEGER                    -- 5 highest .. 1 lowest
);
CREATE INDEX tasks_file   ON tasks (file_id, line);
CREATE INDEX tasks_status ON tasks (status);
CREATE INDEX tasks_due    ON tasks (due) WHERE due IS NOT NULL;

-- Each note's text as last read, which every row above was derived from. A
-- write to a note no tab holds, like a task checked in a view, is made to this
-- text and merged with the file if it changed since.
CREATE TABLE texts (
    file_id INTEGER PRIMARY KEY REFERENCES files (id) ON DELETE CASCADE,
    text    TEXT NOT NULL
);

-- Full text. rowid = files.id. The body is stored (not contentless) so snippet()
-- and highlight() work; at 10k notes that costs a few tens of MB.
-- prefix indexes make search-as-you-type and autocomplete fast.
CREATE VIRTUAL TABLE fts USING fts5 (
    title,
    body,
    tags,
    path,
    tokenize = 'unicode61 remove_diacritics 2',
    prefix = '2 3'
);

-- Stable read surface for the view-block DSL. The query compiler targets these
-- views and `fts` only, so the tables above can change without breaking blocks
-- in notes.
CREATE VIEW v_notes AS
SELECT id,
       path,
       parent AS folder,
       -- file name without '.md': everything after the last '/'
       substr(path, length(rtrim(path, replace(path, '/', ''))) + 1,
              length(path) - length(rtrim(path, replace(path, '/', ''))) - 3) AS name,
       title,
       mtime_ns,
       -- unix seconds; a racy mtime is stored as 0, and the note was indexed
       -- within two seconds of its change
       CASE WHEN mtime_ns > 0 THEN mtime_ns / 1000000000 ELSE indexed_at END AS modified,
       size,
       cloud
FROM files
WHERE ext = 'md';

CREATE VIEW v_tags AS
SELECT file_id, tag, line FROM tags;

CREATE VIEW v_properties AS
SELECT file_id, key, ord, value, value_num FROM frontmatter;

CREATE VIEW v_tasks AS
SELECT t.id,
       t.file_id,
       f.path,
       f.title                                  AS note_title,
       t.line,
       t.indent,
       t.state,
       t.state IN ('x', 'X')                    AS is_done,
       t.status,                                -- a Kanban view maps is_done to its last column
       t.text,
       t.raw,
       t.due,
       t.scheduled,
       t.start,
       t.done_on,
       t.priority
FROM tasks t
JOIN files f ON f.id = t.file_id;

CREATE VIEW v_backlinks AS
SELECT l.dst_file_id AS file_id,
       s.id          AS src_file_id,
       s.path        AS src_path,
       s.title       AS src_title,
       l.line,
       l.alias,
       l.is_embed
FROM links l
JOIN files s ON s.id = l.src_file_id
WHERE l.dst_file_id IS NOT NULL;

-- Typed documents: notes whose frontmatter has a `type` (task, project,
-- meeting, person, ...). `doc_id` is the frontmatter `id`, else the file name;
-- `[[doc_id]]` links to the document either way. `status` is the slug the
-- indexer stores with `sage_core::status_slug`, as Kanban columns are named,
-- with `_` read as `-`: `In Progress` and `in_progress` read as `in-progress`.
-- `tags` are the frontmatter tags, lowercase, comma-separated.
CREATE VIEW v_documents AS
SELECT n.id,
       n.path,
       n.name,
       n.title,
       lower(trim(ty.value))                                     AS type,
       coalesce((SELECT value FROM frontmatter
                 WHERE file_id = n.id AND key = 'id' AND ord = 0), n.name) AS doc_id,
       (SELECT status_key FROM files WHERE id = n.id)            AS status,
       (SELECT value FROM frontmatter
        WHERE file_id = n.id AND key = 'created' AND ord = 0)    AS created,
       (SELECT value FROM frontmatter
        WHERE file_id = n.id AND key = 'due' AND ord = 0)        AS due,
       (SELECT group_concat(tag, ',') FROM (SELECT tag FROM tags
        WHERE file_id = n.id AND line = 0 ORDER BY rowid))       AS tags
FROM v_notes n
JOIN frontmatter ty ON ty.file_id = n.id AND ty.key = 'type' AND ty.ord = 0
WHERE ty.value IS NOT NULL AND trim(ty.value) <> '';
