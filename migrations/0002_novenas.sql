CREATE TYPE novena_mode AS ENUM ('DM','Channel');

CREATE TABLE IF NOT EXISTS novenas (
    id BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    destination BIGINT NOT NULL,
    mode novena_mode NOT NULL,
    title TEXT NOT NULL,
    start_time TIMESTAMP WITH TIME ZONE NOT NULL,
    next_update TIMESTAMP WITH TIME ZONE NOT NULL,
    novena_text TEXT[9] NOT NULL
);
