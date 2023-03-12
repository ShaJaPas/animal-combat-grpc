-- Add up migration script here
CREATE TABLE players
(
    id SERIAL PRIMARY KEY,
    hashed_password CHARACTER VARYING(133) NOT NULL,
    email CHARACTER VARYING(255) UNIQUE NOT NULL,
    nickname CHARACTER VARYING(20) UNIQUE NULL,
    xp INTEGER NOT NULL DEFAULT 0,
    coins INTEGER NOT NULL DEFAULT 0,
    crystals INTEGER NOT NULL DEFAULT 0,
    glory INTEGER NOT NULL DEFAULT 0,
    clan_id INTEGER NULL,
    refresh_token TEXT NOT NULL
)