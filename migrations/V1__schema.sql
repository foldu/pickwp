CREATE TABLE relative_path(
    id INTEGER PRIMARY KEY,
    file_path TEXT NOT NULL UNIQUE
);

CREATE TABLE tag(
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE path_tag(
    relative_path_id INTEGER REFERENCES relative_path(id),
    tag_id INTEGER REFERENCES tag(id),
    PRIMARY KEY (relative_path_id, tag_id)
);