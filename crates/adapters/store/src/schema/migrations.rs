//! The migrations themselves: migration `n` takes the schema from version `n` to `n + 1`.
//! Never edit a shipped one; add the next.

/// Every migration, oldest first.
pub(crate) const MIGRATIONS: &[&str] = &[
    // 1: the session.
    "
    CREATE TABLE meta (
        key TEXT PRIMARY KEY,
        value INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE tabs (
        id INTEGER PRIMARY KEY,
        position INTEGER NOT NULL,
        url TEXT NOT NULL,
        title TEXT NOT NULL,
        jar INTEGER
    ) STRICT;
    CREATE TABLE windows (
        id INTEGER PRIMARY KEY,
        position INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE window_tabs (
        window INTEGER NOT NULL REFERENCES windows(id) ON DELETE CASCADE,
        rank INTEGER NOT NULL,
        tab INTEGER NOT NULL,
        PRIMARY KEY (window, rank)
    ) STRICT;
    CREATE TABLE slots (
        slot INTEGER PRIMARY KEY,
        tab INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE trash (
        position INTEGER PRIMARY KEY,
        id INTEGER NOT NULL,
        url TEXT NOT NULL,
        title TEXT NOT NULL,
        jar INTEGER,
        closed_at INTEGER NOT NULL
    ) STRICT;
",
    // 2: the history, and which parts of the library were ever saved. An entry keeps only its
    // newest visit times, as many as the palette's frecency samples, beside its full count.
    "
    CREATE TABLE library_saved (
        part TEXT PRIMARY KEY
    ) STRICT;
    CREATE TABLE history (
        position INTEGER PRIMARY KEY,
        url TEXT NOT NULL UNIQUE,
        title TEXT NOT NULL,
        visit_count INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE history_visits (
        entry INTEGER NOT NULL REFERENCES history(position) ON DELETE CASCADE,
        rank INTEGER NOT NULL,
        visited_at INTEGER NOT NULL,
        PRIMARY KEY (entry, rank)
    ) STRICT;
",
    // 3: the bookmarks and their tags.
    "
    CREATE TABLE bookmarks (
        position INTEGER PRIMARY KEY,
        url TEXT NOT NULL UNIQUE,
        title TEXT NOT NULL,
        saved_at INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE bookmark_tags (
        bookmark INTEGER NOT NULL REFERENCES bookmarks(position) ON DELETE CASCADE,
        tag TEXT NOT NULL,
        PRIMARY KEY (bookmark, tag)
    ) STRICT;
",
    // 4: the search shortcuts.
    "
    CREATE TABLE search_shortcuts (
        position INTEGER PRIMARY KEY,
        keyword TEXT NOT NULL UNIQUE,
        template TEXT NOT NULL
    ) STRICT;
",
    // 5: the user's key bindings, one `keybindings.json` entry per row, as written.
    "
    CREATE TABLE keybindings (
        position INTEGER PRIMARY KEY,
        key TEXT NOT NULL,
        command TEXT NOT NULL,
        when_clause TEXT
    ) STRICT;
",
    // 6: the shields: per-site exceptions, the filter lists' states, when lists are refreshed,
    // and each list's content as last fetched. A list is fetched at a time with a hash, or not
    // at all.
    "
    CREATE TABLE site_policies (
        position INTEGER PRIMARY KEY,
        site TEXT NOT NULL UNIQUE,
        shields INTEGER NOT NULL,
        https_only INTEGER NOT NULL,
        javascript INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE filter_lists (
        id TEXT PRIMARY KEY,
        enabled INTEGER NOT NULL,
        fetched_at INTEGER,
        content_hash INTEGER,
        CHECK ((fetched_at IS NULL) = (content_hash IS NULL))
    ) STRICT;
    CREATE TABLE filter_refresh (
        keep_fresh INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE filter_list_bodies (
        id TEXT PRIMARY KEY,
        body BLOB NOT NULL
    ) STRICT;
",
    // 7: the agent's endpoint, in at most one row. `key_ref` names the API key in the Keychain;
    // the key itself is never stored here.
    "
    CREATE TABLE agent_endpoint (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        base_url TEXT NOT NULL,
        model TEXT NOT NULL,
        key_ref TEXT
    ) STRICT;
",
];
