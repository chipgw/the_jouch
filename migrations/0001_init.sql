CREATE TYPE birthday_privacy AS ENUM ('PublicFull','PublicDay','Private');

CREATE TABLE IF NOT EXISTS users (
    user_id BIGINT,
    guild_id BIGINT,
    PRIMARY KEY (user_id, guild_id),
    birthday TIMESTAMP WITH TIME ZONE,
    birthday_privacy birthday_privacy,
    auto_nick TEXT,
    sit_count INT NOT NULL DEFAULT 0,
    flip_count INT NOT NULL DEFAULT 0
);

CREATE TYPE jouch_orientation AS ENUM ('Normal','UpsideDown','RotatedLeft','RotatedRight');

CREATE TABLE IF NOT EXISTS guilds (
    id BIGINT PRIMARY KEY,
    birthday_announce_channel BIGINT,
    birthday_announce_when_none BOOLEAN,
    canned_response_table JSON,
    jouch_orientation jouch_orientation NOT NULL DEFAULT 'Normal'
);

CREATE TABLE IF NOT EXISTS config (
    id INT PRIMARY KEY GENERATED ALWAYS AS (1) STORED,
    nick_interval BIGINT,
    canned_response_table JSON
);
