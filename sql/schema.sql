CREATE TABLE root (
    id INTEGER PRIMARY KEY,
    root_path TEXT UNIQUE NOT NULL
);

CREATE TABLE relative_path (
    id INTEGER PRIMARY KEY,
    root_id INTEGER REFERENCES root(id) NOT NULL,
    file_path TEXT NOT NULL,
    unix_mtime INTEGER NOT NULL,
    -- not every file system supports btime
    unix_btime INTEGER,
    UNIQUE (root_id, file_path)
);

CREATE TABLE tag (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE path_tag (
    relative_path_id INTEGER REFERENCES relative_path(id),
    tag_id INTEGER REFERENCES tag(id),
    PRIMARY KEY (relative_path_id, tag_id)
);

CREATE TABLE history (
    id INTEGER PRIMARY KEY,
    -- could use datetime but sqlite's datetime is weird
    unix_timestamp INTEGER NOT NULL,
    relative_path_id INTEGER NOT NULL REFERENCES relative_path(id) ON DELETE CASCADE
);
