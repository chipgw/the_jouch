CREATE TABLE IF NOT EXISTS reaction_tracking (
    user_id BIGINT,
    guild_id BIGINT,
    reaction TEXT,
    PRIMARY KEY (user_id, guild_id, reaction),
    count INT NOT NULL DEFAULT 0,
    first_seen TIMESTAMP NOT NULL DEFAULT NOW()
);
