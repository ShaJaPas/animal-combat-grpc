-- Add up migration script here
CREATE TABLE players
(
    id SERIAL PRIMARY KEY,
    hashed_password CHARACTER VARYING(133) NOT NULL,
    email CHARACTER VARYING(30) UNIQUE NOT NULL,
    refresh_token TEXT NOT NULL
)