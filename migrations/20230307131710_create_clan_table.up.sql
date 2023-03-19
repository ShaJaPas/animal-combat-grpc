-- Add up migration script here
CREATE TYPE clan_type as ENUM ('Open', 'Closed', 'InviteOnly');

CREATE TABLE clans
(
    id SERIAL PRIMARY KEY,
    clan_name CHARACTER VARYING(20) UNIQUE NOT NULL,
    description CHARACTER VARYING(80) NULL,
    min_glory INTEGER NOT NULL DEFAULT 0 CHECK (MOD(min_glory, 300) = 0),
    max_members INTEGER NOT NULL,
    type clan_type NOT NULL,
    creator_id INTEGER NOT NULL
);

ALTER TABLE players ADD CONSTRAINT fk_clan FOREIGN KEY(clan_id) REFERENCES clans(id);
ALTER TABLE clans ADD CONSTRAINT fk_player FOREIGN KEY(creator_id) REFERENCES players(id);