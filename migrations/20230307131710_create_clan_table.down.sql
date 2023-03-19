-- Add down migration script here
ALTER TABLE players DROP CONSTRAINT fk_clan;
ALTER TABLE clans DROP CONSTRAINT fk_player;

DROP TABLE clans;

DROP TYPE clan_type;