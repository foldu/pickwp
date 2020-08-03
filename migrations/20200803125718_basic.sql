CREATE TABLE root (
    id INTEGER PRIMARY KEY NOT NULL,
    root_path TEXT UNIQUE NOT NULL
);

CREATE TABLE relative_path (
    id INTEGER PRIMARY KEY NOT NULL,
    root_id INTEGER REFERENCES root(id) NOT NULL,
    file_path TEXT NOT NULL,
    unix_mtime INTEGER NOT NULL,
    -- not every file system supports btime
    unix_btime INTEGER,
    UNIQUE (root_id, file_path)
);

CREATE TABLE tag (
    id INTEGER PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE path_tag (
    relative_path_id INTEGER REFERENCES relative_path(id) NOT NULL,
    tag_id INTEGER REFERENCES tag(id) NOT NULL,
    PRIMARY KEY (relative_path_id, tag_id)
);

CREATE TABLE history (
    id INTEGER PRIMARY KEY NOT NULL,
    -- could use datetime but sqlite's datetime is weird
    unix_timestamp INTEGER NOT NULL,
    relative_path_id INTEGER NOT NULL REFERENCES relative_path(id) ON DELETE CASCADE
);
