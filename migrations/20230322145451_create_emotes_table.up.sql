-- Add up migration script here
CREATE TABLE emotes 
(
    id SERIAL PRIMARY KEY,
    file_name CHARACTER VARYING(30) UNIQUE NOT NULL
);

INSERT INTO emotes 
  (file_name) 
VALUES 
  ('devil'),
  ('evil'),
  ('giggle'),
  ('grimacing'),
  ('grinning'),
  ('heart_eyes'),
  ('ill'),
  ('laughing'),
  ('mind_blow'),
  ('sad'),
  ('shock'),
  ('shout'),
  ('sigh'),
  ('silly'),
  ('star_eyes'),
  ('sunglasses'),
  ('swearing'),
  ('sweat'),
  ('thinking'),
  ('worried'),
  ('happy');

CREATE TABLE players_emotes (
  player_id INTEGER REFERENCES players (id) ON UPDATE CASCADE ON DELETE CASCADE,
  emote_id int REFERENCES emotes (id) ON UPDATE CASCADE,
  CONSTRAINT players_emotes_pkey PRIMARY KEY (player_id, emote_id)
);