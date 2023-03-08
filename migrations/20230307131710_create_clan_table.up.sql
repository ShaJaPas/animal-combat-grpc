-- Add up migration script here
CREATE TABLE clans
(
    id SERIAL PRIMARY KEY,
    clan_name CHARACTER VARYING(20) UNIQUE NOT NULL,
    max_members INTEGER NOT NULL
);

ALTER TABLE players ADD CONSTRAINT fk_clan FOREIGN KEY(clan_id) REFERENCES clans(id);