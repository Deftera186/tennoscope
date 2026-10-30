PRAGMA foreign_keys = ON;

CREATE TABLE inventory (
    item_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    category TEXT NOT NULL,
    quantity INTEGER NOT NULL CHECK (quantity >= 0),
    mastered INTEGER NOT NULL CHECK (mastered IN (0, 1)),
    image_name TEXT,
    rank INTEGER CHECK (rank IS NULL OR rank >= 0),
    max_rank INTEGER CHECK (max_rank IS NULL OR max_rank >= 0)
);

CREATE TABLE market_credential (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    token TEXT NOT NULL CHECK (length(trim(token)) > 0)
);

CREATE TABLE snapshot_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    observed_at TEXT NOT NULL CHECK (length(trim(observed_at)) > 0),
    game_build TEXT NOT NULL CHECK (length(trim(game_build)) > 0),
    source TEXT NOT NULL CHECK (length(trim(source)) > 0),
    item_count INTEGER NOT NULL CHECK (item_count >= 0)
);
