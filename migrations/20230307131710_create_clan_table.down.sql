-- Add down migration script here
ALTER TABLE players DROP CONSTRAINT fk_clan;

DROP TABLE clans;
