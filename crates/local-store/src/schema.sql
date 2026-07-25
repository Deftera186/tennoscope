PRAGMA foreign_keys = ON;

CREATE TABLE inventory (
    item_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    category TEXT NOT NULL,
    quantity INTEGER NOT NULL CHECK (quantity >= 0),
    mastered INTEGER NOT NULL CHECK (mastered IN (0, 1)),
    image_name TEXT
);

CREATE TABLE snapshot_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    observed_at TEXT NOT NULL CHECK (length(trim(observed_at)) > 0),
    game_build TEXT NOT NULL CHECK (length(trim(game_build)) > 0),
    source TEXT NOT NULL CHECK (length(trim(source)) > 0),
    item_count INTEGER NOT NULL CHECK (item_count >= 0)
);
