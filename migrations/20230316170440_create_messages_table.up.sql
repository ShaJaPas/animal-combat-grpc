-- Add up migration script here
CREATE TYPE message_type as ENUM ('SystemPositive', 'SystemNegative', 'Player');

CREATE TABLE messages
(
    id SERIAL PRIMARY KEY,
    player_id INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    content TEXT NOT NULL,
    msg_type message_type NOT NULL
);