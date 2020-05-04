CREATE TABLE relative_path (
    id INTEGER PRIMARY KEY,
    file_path TEXT NOT NULL UNIQUE,
    unix_mtime INTEGER NOT NULL,
    -- not every file system supports btime
    unix_btime INTEGER
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
